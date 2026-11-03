//! The dependency graph: recompute order and evaluation.

use super::*;

impl Engine {
    /// `names` with each one after the others it reads, directly or through
    /// formulas; otherwise in the order given.
    pub(super) fn in_dependency_order(&self, names: &[String]) -> Vec<String> {
        let wanted: HashSet<&str> = names.iter().map(String::as_str).collect();
        let mut done = HashSet::new();
        let mut out = Vec::with_capacity(names.len());
        fn visit<'a>(
            e: &'a Engine,
            name: &'a str,
            wanted: &HashSet<&str>,
            done: &mut HashSet<&'a str>,
            out: &mut Vec<String>,
        ) {
            if !done.insert(name) {
                return;
            }
            if let Some(c) = e.cells.get(name) {
                for dep in &c.deps {
                    let through = e
                        .cells
                        .get(dep.as_str())
                        .is_some_and(|d| d.kind == CellKind::Expr);
                    if wanted.contains(dep.as_str()) || through {
                        visit(e, dep, wanted, done, out);
                    }
                }
            }
            if wanted.contains(name) {
                out.push(name.to_string());
            }
        }
        for name in names {
            visit(self, name, &wanted, &mut done, &mut out);
        }
        out
    }

    pub fn dependents_of(&self, name: &str) -> Vec<String> {
        self.dependents
            .get(name)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Mark everything downstream of `roots` dirty and recompute it in
    /// topological order, roots included.
    pub(super) fn recompute(&mut self, roots: Vec<String>) -> Result<()> {
        // Dirty closure.
        let mut dirty: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for root in roots {
            if seen.insert(root.clone()) {
                dirty.push(root);
            }
        }
        let mut i = 0;
        while i < dirty.len() {
            for dep in self.dependents_of(&dirty[i]) {
                if seen.insert(dep.clone()) {
                    dirty.push(dep);
                }
            }
            i += 1;
        }

        // Kahn's algorithm. External def writes can smuggle a cycle past install;
        // the cells left over are that cycle.
        let mut indegree: HashMap<&str, usize> = dirty
            .iter()
            .map(|n| {
                let n = n.as_str();
                let deg = self
                    .cells
                    .get(n)
                    .map_or(0, |c| c.deps.iter().filter(|d| seen.contains(*d)).count());
                (n, deg)
            })
            .collect();
        let mut queue: Vec<String> = dirty
            .iter()
            .filter(|n| indegree[n.as_str()] == 0)
            .cloned()
            .collect();
        let mut order = Vec::new();
        while let Some(n) = queue.pop() {
            for dep in self.dependents_of(&n) {
                if let Some(deg) = indegree.get_mut(dep.as_str()) {
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push(dep);
                    }
                }
            }
            order.push(n);
        }
        if order.len() < dirty.len() {
            for name in dirty.iter().filter(|n| !order.contains(n)) {
                if let Some(cell) = self.cells.get_mut(name) {
                    cell.state = CellState::Error("cycle".into());
                    self.store.set_state(cell)?;
                    self.touched.insert(name.clone());
                }
            }
        }

        for name in order {
            self.recompute_cell(&name)?;
        }
        Ok(())
    }

    /// What a formula reads for `dep`: a cell's value, or with a suffix
    /// (`B2.code`, see `expr::META`) one of its run or review facts.
    pub fn formula_lookup(&self, dep: &str) -> String {
        let (name, meta) = expr::split_meta(dep);
        let Some(c) = self.cells.get(name) else {
            return String::new();
        };
        match meta {
            // A computed cell with no answer yet, or none because its run
            // failed: the formula reads it as blank, except as a number.
            None if c.kind != CellKind::Literal && c.value.is_empty() => match c.state {
                CellState::Stale | CellState::Computing | CellState::Queued => expr::PENDING.into(),
                CellState::Error(_) => expr::FAILED.into(),
                _ => String::new(),
            },
            None => c.value.clone(),
            Some("code") => c.exit_code.map(|v| v.to_string()).unwrap_or_default(),
            // Trimmed as a value's stdout is.
            Some("err") => c.stderr.trim_end().to_string(),
            Some("dur") => c.duration_ms.map(|v| v.to_string()).unwrap_or_default(),
            Some("cost") => c
                .usage
                .as_ref()
                .and_then(|u| u.cost_usd)
                .map(|v| v.to_string())
                .unwrap_or_default(),
            Some("model") => c
                .usage
                .as_ref()
                .and_then(|u| u.model.clone())
                .unwrap_or_default(),
            Some("by") => String::from(c.provenance.by.clone()),
            Some("mark") => match c.mark.as_ref().map(|m| m.kind) {
                Some(crate::model::MarkKind::Verified) => "verified".into(),
                Some(crate::model::MarkKind::Flagged) => "flagged".into(),
                None => String::new(),
            },
            Some("cited") => c
                .cite
                .as_ref()
                .is_some_and(|ct| ct.found != Some(false))
                .to_string(),
            Some(_) => String::new(),
        }
    }

    pub(super) fn recompute_cell(&mut self, name: &str) -> Result<()> {
        let Some(cell) = self.cells.get(name) else {
            return Ok(());
        };
        if cell.state == CellState::Awaiting {
            return Ok(()); // a pending decision outranks recomputation
        }
        match cell.kind {
            // Literals only change by direct write.
            CellKind::Literal => Ok(()),
            CellKind::Expr => {
                // Never panic on a cell read halfway through another writer's
                // change; its next write settles it.
                let Some(source) = cell.source.clone() else {
                    return Ok(());
                };
                // Record what the formula reads: a named range's members are deps only
                // evaluation discovers, and an untaken branch isn't one.
                let read = std::cell::RefCell::new(Vec::new());
                let result = expr::parse(def_body(&source)).and_then(|ast| {
                    expr::eval(&ast, &|dep| {
                        let dep = self.qualify(dep, Some(name));
                        let mut read = read.borrow_mut();
                        let base = expr::split_meta(&dep).0;
                        if !read.iter().any(|d| d == base) {
                            read.push(base.to_string());
                        }
                        self.formula_lookup(&dep)
                    })
                });
                let read = read.into_inner();
                // A number it needs isn't in yet: wait for it, rather than
                // compute from a blank (see `expr::PENDING`).
                let held =
                    matches!(&result, Err(e) if e == expr::WAITING).then_some(CellState::Stale);
                match (result, held) {
                    (Ok(value), None) => {
                        let mut cell = self.cells[name].clone();
                        cell.value = value;
                        cell.deps = read;
                        cell.state = CellState::Ok;
                        cell.provenance = Provenance::now(Actor::Engine);
                        self.store.update(&cell)?;
                        self.put(cell);
                        Ok(())
                    }
                    (result, held) => {
                        // Keep the last good value, and add what this eval read before it
                        // failed, or the input it failed on could never wake it again.
                        let mut cell = self.cells[name].clone();
                        for dep in read {
                            if !cell.deps.contains(&dep) {
                                cell.deps.push(dep);
                            }
                        }
                        cell.state = held
                            .unwrap_or_else(|| CellState::Error(result.err().unwrap_or_default()));
                        cell.provenance = Provenance::now(Actor::Engine);
                        self.store.set_state(&cell)?;
                        // `put`, so the dependents index learns the new inputs.
                        self.put(cell);
                        Ok(())
                    }
                }
            }
            CellKind::Command => {
                if !self.active {
                    // The active engine runs it; the result arrives through the watch.
                    return Ok(());
                }
                let Some(source) = cell.source.as_deref() else {
                    return Ok(());
                };
                if let Some(e) = source_error(CellKind::Command, def_body(source)) {
                    // Installed as Error; a dep change must not run it, and the wave marked it
                    // stale, so restore the reason.
                    let cell = self.cells.get_mut(name).expect("checked above");
                    if cell.state != CellState::Error(e.clone()) {
                        cell.state = CellState::Error(e);
                        self.store.set_state(cell)?;
                    }
                    return Ok(());
                }
                if !cell.confirmed && danger::scan(def_body(source), &self.config.danger_tokens) {
                    let cell = self.cells.get_mut(name).expect("checked above");
                    if cell.state != CellState::Quarantined {
                        cell.state = CellState::Quarantined;
                        self.store.set_state(cell)?;
                        self.touched.insert(name.to_string());
                    }
                    return Ok(());
                }
                self.spawn_run(name)
            }
            // `!` and `@` never auto-run: dirtying marks them Stale. An agent call that
            // doesn't parse stays an error.
            CellKind::Action | CellKind::AgentCall => {
                let cell = self.cells.get_mut(name).expect("checked above");
                // A run in progress or queued lands and dirties dependents itself.
                if matches!(cell.state, CellState::Computing | CellState::Queued) {
                    return Ok(());
                }
                let source = cell.source.as_deref().expect("has a source");
                let next = match cell.kind {
                    CellKind::AgentCall => match agent::parse(def_body(source)) {
                        Err(e) => CellState::Error(e),
                        Ok(_) => CellState::Stale,
                    },
                    // A `! cmd # watch:` stays an error, not "needs run".
                    _ => match source_error(CellKind::Action, def_body(source)) {
                        Some(e) => CellState::Error(e),
                        None => CellState::Stale,
                    },
                };
                if cell.state != next {
                    cell.state = next;
                    self.store.set_state(cell)?;
                    self.touched.insert(name.to_string());
                }
                Ok(())
            }
        }
    }

    /// `interp_lookup` for a reference written in cell `host`.
    pub(super) fn interp_lookup_from(&self, host: &str, written: &str, form: Form) -> String {
        match form {
            // Board-relative paths and patterns, not cells.
            Form::RepoFile | Form::RepoGlob => self.interp_lookup(written, form),
            _ => self.interp_lookup(&self.qualify(written, Some(host)), form),
        }
    }

    pub(super) fn interp_lookup(&self, name: &str, form: Form) -> String {
        let cell = self.cells.get(name);
        match form {
            Form::Quoted | Form::Raw => cell.map(|c| c.value.clone()).unwrap_or_default(),
            Form::File => self.store.value_path(name).to_string_lossy().into_owned(),
            Form::Code => cell
                .and_then(|c| c.exit_code)
                .map(|c| c.to_string())
                .unwrap_or_default(),
            Form::Err => cell.map(|c| c.stderr.clone()).unwrap_or_default(),
            Form::Dur => cell
                .and_then(|c| c.duration_ms)
                .map(|d| d.to_string())
                .unwrap_or_default(),
            // Repo refs: a board-relative path or glob, made absolute so the cwd
            // doesn't matter.
            Form::RepoFile => self.launch_dir.join(name).to_string_lossy().into_owned(),
            Form::RepoGlob => self.glob_matches(name),
            // Expanded per member by `interp::resolve` and `agent::task_text`.
            Form::Range => String::new(),
        }
    }

    /// The repo files matching a `{glob:}` ref right now, as absolute paths,
    /// sorted, shell-quoted and space-joined for an argv.
    pub(super) fn glob_matches(&self, pattern: &str) -> String {
        // Only a cell that failed install validation has a bad pattern; it matches
        // nothing.
        let Ok(glob) = globset::Glob::new(pattern) else {
            return String::new();
        };
        let matcher = glob.compile_matcher();
        let mut start = self.launch_dir.join(glob_root(pattern));
        while !start.exists() {
            let Some(parent) = start.parent() else { break };
            start = parent.to_path_buf();
        }
        let mut hits = Vec::new();
        if start.is_file() {
            // A fully-literal pattern roots at the file itself.
            if glob_root(pattern) == Path::new(pattern) {
                hits.push(start);
            }
        } else {
            walk_matches(&start, &self.launch_dir, &matcher, &mut hits);
        }
        hits.sort();
        let quoted: Vec<String> = hits
            .iter()
            .map(|p| interp::shell_quote(&p.to_string_lossy()))
            .collect();
        quoted.join(" ")
    }
}

/// Collect files under `dir` whose path relative to `root` matches. Skips
/// `.recalc/` and `.git/` unless the walk starts inside one.
pub(super) fn walk_matches(
    dir: &Path,
    root: &Path,
    m: &globset::GlobMatcher,
    out: &mut Vec<PathBuf>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name == ".recalc" || name == ".git" {
            continue;
        }
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            walk_matches(&path, root, m, out);
        } else if path.strip_prefix(root).is_ok_and(|rel| m.is_match(rel)) {
            out.push(path);
        }
    }
}

/// The literal directory prefix of a glob, the deepest path that can be
/// watched (`src/**/*.rs` → `src`).
pub(super) fn glob_root(glob: &str) -> PathBuf {
    let mut root = PathBuf::new();
    for part in glob.split('/') {
        if part.contains(['*', '?', '[', '{']) {
            break;
        }
        root.push(part);
    }
    root
}
