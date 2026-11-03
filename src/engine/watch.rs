//! Re-running `$` cells when watched files change or a period elapses.

use super::*;

impl Engine {
    /// Re-run `$` cells whose watch glob or `{file:}`/`{glob:}` ref matched a
    /// file change, or whose `# every:` period elapsed. The usual gates apply.
    pub(super) fn pump_watched(&mut self) -> Result<bool> {
        if self.watch_dirty {
            self.watch_dirty = false;
            self.rebuild_watches();
        }

        let mut due: Vec<String> = Vec::new();
        if self.active && !std::mem::replace(&mut self.watches_caught_up, true) {
            due = self.changed_while_unwatched();
        }
        if let Some(w) = &mut self.repo_watch {
            let mut paths: Vec<(PathBuf, bool)> = Vec::new();
            while let Ok(p) = w.rx.try_recv() {
                if !paths.iter().any(|(q, _)| *q == p) {
                    let in_git = p.components().any(|c| c.as_os_str() == ".git");
                    paths.push((p, in_git));
                }
            }
            for m in &w.matchers {
                // .git paths only reach matchers that target them, so `**` doesn't fire
                // on every git write.
                let hit = paths.iter().any(|(p, in_git)| {
                    // Linux reports the watched root itself; its relative path is
                    // empty, and `**` matches that.
                    (!in_git || m.git)
                        && p.strip_prefix(&w.root)
                            .is_ok_and(|rel| !rel.as_os_str().is_empty() && m.glob.is_match(rel))
                });
                if hit && !due.contains(&m.cell) {
                    due.push(m.cell.clone());
                }
            }
        }

        let now = std::time::Instant::now();
        let everies = &self.everies;
        self.every_due
            .retain(|name, _| everies.iter().any(|(n, _)| n == name));
        for (name, ms) in self.everies.clone() {
            let period = std::time::Duration::from_millis(ms);
            // A new entry starts one period out: install already ran the cell.
            let due_at = self.every_due.entry(name.clone()).or_insert(now + period);
            if now >= *due_at {
                *due_at = now + period;
                if !due.contains(&name) {
                    due.push(name);
                }
            }
        }

        let moved = !due.is_empty();
        for name in due {
            self.recompute_cell(&name)?;
        }
        Ok(moved)
    }

    /// Re-run watched `$` cells whose files changed since they last ran. For a
    /// command-line engine, which has no watcher; a window's engine does this
    /// as files change, and a passive engine leaves it to that one.
    pub fn rerun_changed_checks(&mut self) -> Result<()> {
        if !self.active {
            return Ok(());
        }
        if std::mem::take(&mut self.watch_dirty) {
            self.rebuild_watches();
        }
        for name in self.changed_while_unwatched() {
            self.recompute_cell(&name)?;
        }
        Ok(())
    }

    /// Watched `$` cells whose files changed after the cell last ran, while no
    /// engine was watching.
    pub(super) fn changed_while_unwatched(&self) -> Vec<String> {
        let Some(w) = &self.repo_watch else {
            return Vec::new();
        };
        let mut due = Vec::new();
        for (cell, pattern) in &w.specs {
            if due.contains(cell) {
                continue;
            }
            let Some(c) = self.cells.get(cell) else {
                continue;
            };
            if matches!(c.state, CellState::Computing | CellState::Queued) {
                continue;
            }
            let last = std::time::UNIX_EPOCH + std::time::Duration::from_millis(c.provenance.at);
            let Ok(glob) = globset::Glob::new(pattern) else {
                continue;
            };
            let matcher = glob.compile_matcher();
            let mut start = self.launch_dir.join(glob_root(pattern));
            while !start.exists() {
                let Some(parent) = start.parent() else { break };
                start = parent.to_path_buf();
            }
            let mut files = Vec::new();
            if start.is_file() {
                files.push(start);
            } else {
                walk_matches(&start, &self.launch_dir, &matcher, &mut files);
            }
            let newer = files.iter().any(|f| {
                std::fs::metadata(f)
                    .and_then(|m| m.modified())
                    .is_ok_and(|t| t > last)
            });
            if newer {
                due.push(cell.clone());
            }
        }
        due
    }

    /// Re-derive the watch and interval specs from the `$` cells, and
    /// replace the OS watcher when the (cell, glob) set changed.
    pub(super) fn rebuild_watches(&mut self) {
        let mut watches: Vec<(String, String)> = Vec::new();
        let mut everies: Vec<(String, u64)> = Vec::new();
        for (name, cmd, ann) in self.annotated_commands() {
            for glob in ann.watch.into_iter().flat_map(str::split_whitespace) {
                watches.push((name.to_string(), glob.to_string()));
            }
            for (pattern, is_glob) in interp::repo_refs(cmd) {
                let glob = if is_glob {
                    pattern
                } else {
                    globset::escape(&pattern)
                };
                watches.push((name.to_string(), glob));
            }
            if let Ok(Some(ms)) = ann.every_ms() {
                everies.push((name.to_string(), ms));
            }
        }
        watches.sort();
        watches.dedup();
        self.everies = everies;

        let stale = match &self.repo_watch {
            None => !watches.is_empty(),
            Some(w) => w.specs != watches,
        };
        if !stale {
            return;
        }
        self.repo_watch = if watches.is_empty() {
            None
        } else {
            Some(match self.build_repo_watch(&watches) {
                Ok(w) => w,
                Err(e) => {
                    self.log(format!("watch failed: {e}"));
                    let (_, rx) = mpsc::unbounded_channel();
                    RepoWatch {
                        specs: watches,
                        matchers: Vec::new(),
                        root: self.launch_dir.clone(),
                        rx,
                        _watcher: None,
                    }
                }
            })
        };
    }

    /// The `$` cells with a valid source, as (name, command, annotations).
    pub(super) fn annotated_commands(&self) -> impl Iterator<Item = (&str, &str, Annotations<'_>)> {
        self.cells.values().filter_map(|cell| {
            if cell.kind != CellKind::Command {
                return None;
            }
            let body = def_body(cell.source.as_deref()?);
            let (cmd, ann) = split_annotations(body);
            source_error(CellKind::Command, body).is_none().then_some((
                cell.name.as_str(),
                cmd,
                ann,
            ))
        })
    }

    /// Build the notify watcher. Each glob is watched from its literal prefix
    /// (or nearest existing ancestor), so `src/**` never watches target/.
    pub(super) fn build_repo_watch(&self, specs: &[(String, String)]) -> Result<RepoWatch> {
        let root = self.launch_dir.canonicalize()?;
        let mut matchers = Vec::new();
        let mut roots: Vec<PathBuf> = Vec::new();
        for (name, glob) in specs {
            // Globs were validated at install, so this compile succeeds.
            let m = globset::Glob::new(glob)?.compile_matcher();
            matchers.push(Matcher {
                cell: name.clone(),
                glob: m,
                git: glob.starts_with(".git"),
            });
            let mut dir = root.join(glob_root(glob));
            while !dir.exists() {
                let Some(parent) = dir.parent() else { break };
                dir = parent.to_path_buf();
            }
            if !roots.contains(&dir) {
                roots.push(dir);
            }
        }
        let (tx, rx) = mpsc::unbounded_channel();
        let mut watcher =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                let Ok(event) = res else { return };
                // Linux reports opening and reading a file; only changes count, or
                // `$ cat f # watch: f` would re-run itself forever.
                if matches!(event.kind, notify::EventKind::Access(_)) {
                    return;
                }
                let created = matches!(event.kind, notify::EventKind::Create(_));
                for path in event.paths {
                    // The board's own writes never retrigger runs, or a `**` glob would loop.
                    // Git paths are filtered per matcher in the pump.
                    let ours = path.components().any(|c| c.as_os_str() == ".recalc");
                    if ours {
                        continue;
                    }
                    // On Linux a new folder is watched only once it's reported, so
                    // files made in it straight away go unseen; report what it holds.
                    if created && path.is_dir() {
                        for file in files_under(&path, NEW_DIR_SCAN) {
                            let _ = tx.send(file);
                        }
                    }
                    let _ = tx.send(path);
                }
            })?;
        for r in &roots {
            watcher.watch(r, notify::RecursiveMode::Recursive)?;
        }
        Ok(RepoWatch {
            specs: specs.to_vec(),
            matchers,
            root,
            rx,
            _watcher: Some(watcher),
        })
    }
}

/// How many entries of a newly made folder are reported, so a folder that
/// arrives full (an unpacked archive) can't flood the channel.
const NEW_DIR_SCAN: usize = 2000;

/// Files under `dir`, up to `limit` entries looked at.
fn files_under(dir: &Path, limit: usize) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut dirs = vec![dir.to_path_buf()];
    let mut seen = 0;
    while let Some(d) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > limit {
                return files;
            }
            let path = entry.path();
            match entry.file_type() {
                Ok(t) if t.is_dir() => dirs.push(path),
                Ok(_) => files.push(path),
                Err(_) => {}
            }
        }
    }
    files
}
