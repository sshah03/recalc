//! Reconciling with writes from other recalc processes and outside edits.

use super::*;

impl Engine {
    /// Reconcile an on-disk change with the mirror; our own writes reconcile to
    /// no-ops.
    ///
    /// A passive engine only mirrors: reading between the active engine's value
    /// and meta writes, it would take an answer for a raw edit and pin it.
    pub(super) fn reconcile(&mut self, name: &str) -> Result<()> {
        self.note_sheet(name);
        let disk = self.store.get(name)?;
        let mirror = self.cells.get(name).cloned();
        if !self.active {
            match (disk, mirror) {
                (None, Some(_)) => self.remove(name),
                (Some(mut disk), mirror) => {
                    // A run this process owns keeps its state; disk may
                    // hold a sibling's older idea of it.
                    if let Some(m) = mirror.filter(|_| self.store.owns_run(name)) {
                        disk.state = m.state;
                    }
                    self.put(disk);
                }
                (None, None) => {}
            }
            return Ok(());
        }
        match (disk, mirror) {
            (None, None) => Ok(()),
            (None, Some(_)) => {
                // Deleted externally; dependents read it as empty.
                self.remove(name);
                let dependents = self.dependents_of(name);
                self.recompute(dependents)
            }
            (Some(disk), None) => {
                self.log_outside_write(&disk);
                if disk.source.is_some() {
                    // A def that appeared from outside.
                    self.reinstall_external(disk)
                } else {
                    self.put(disk);
                    self.recompute(vec![name.to_string()])
                }
            }
            (Some(disk), Some(mirror)) => {
                if disk.source != mirror.source {
                    self.log_outside_write(&disk);
                    if disk.source.is_some() {
                        // A def changed on disk: re-install it as-is.
                        self.reinstall_external(disk)
                    } else {
                        // Def removed externally: adopt the literal.
                        self.bump_gen(name);
                        self.put(disk);
                        let dependents = self.dependents_of(name);
                        self.recompute(dependents)
                    }
                } else if disk.value != mirror.value {
                    // A sibling writes the value file, then its meta; between the two its write
                    // looks raw. Look again.
                    let disk = if mirror.source.is_some() && disk.provenance == mirror.provenance {
                        std::thread::sleep(std::time::Duration::from_millis(150));
                        match self.store.get(name)? {
                            Some(again) => again,
                            None => return self.reconcile(name),
                        }
                    } else {
                        disk
                    };
                    if disk.source != mirror.source {
                        return self.reconcile(name);
                    }
                    if mirror.source.is_some() && disk.provenance == mirror.provenance {
                        // Value changed but meta didn't: a raw write over a computed cell, never
                        // applied silently.
                        self.hold_external_override(mirror, disk.value)
                    } else {
                        // Written with its meta by another engine or the CLI, or a literal:
                        // adopt.
                        if disk.provenance != mirror.provenance {
                            self.log_outside_write(&disk);
                        }
                        self.bump_gen(name);
                        self.put(disk);
                        let dependents = self.dependents_of(name);
                        self.recompute(dependents)
                    }
                } else {
                    // Meta-only change from a sibling: mirror it without recomputing. A run
                    // held here (owned, or handed off and not yet taken) keeps our state, which
                    // is newer than the sibling's.
                    let mut disk = disk;
                    let held_here = self.store.owns_run(name)
                        || (self.store.handoff_depth(name).is_some()
                            && !matches!(disk.state, CellState::Error(_)));
                    if matches!(mirror.state, CellState::Computing | CellState::Queued) && held_here
                    {
                        disk.state = mirror.state;
                    }
                    let review_moved = disk.mark != mirror.mark || disk.cite != mirror.cite;
                    self.put(disk);
                    if review_moved {
                        self.refresh_meta_readers(name)?;
                    }
                    Ok(())
                }
            }
        }
    }

    /// Bring the mirror in line with the disk now, without waiting for the
    /// watch: a move rewrites every cell from the mirror, so a write another
    /// process made a moment ago would be lost.
    pub(super) fn catch_up(&mut self) -> Result<()> {
        self.adopt_sheets()?;
        let mut names: BTreeSet<String> = self.cells.keys().cloned().collect();
        let same =
            |a: &Cell, b: &Cell| serde_json::to_value(a).ok() == serde_json::to_value(b).ok();
        for disk in self.store.list()? {
            if self.cells.get(&disk.name).is_some_and(|m| same(m, &disk)) {
                names.remove(&disk.name);
            } else {
                names.insert(disk.name);
            }
        }
        for name in names {
            self.reconcile(&name)?;
        }
        Ok(())
    }

    /// An agent cell whose terminal session is live (shown in a window, or
    /// carried on by tmux). Sessions are keyed by cell name, so a move would
    /// leave one behind at an address that now holds another cell.
    pub(super) fn live_session(&self) -> Option<String> {
        let tmux = crate::session::available();
        self.cells
            .values()
            .filter(|c| c.kind == CellKind::AgentCall)
            .filter(|c| {
                self.cells
                    .get(&crate::model::status_name(&c.name))
                    .is_some_and(|s| {
                        matches!(s.value.as_str(), "running" | "waiting?" | "detached")
                    })
            })
            // A status left behind by a session that died says nothing.
            .find(|c| {
                !tmux || crate::session::alive(&crate::session::name(&self.launch_dir, &c.name))
            })
            .map(|c| c.name.clone())
    }

    /// Log a write another process made (an agent's MCP server, the command
    /// line) under who made it; a run's result is logged where it finishes.
    fn log_outside_write(&mut self, disk: &Cell) {
        if !matches!(disk.provenance.by, Actor::Engine | Actor::System) {
            let by = disk.provenance.by.clone();
            self.log_as(&by, format!("set {}", disk.name));
        }
    }

    /// An externally edited def. Its source is already on disk, so a cycle
    /// marks the cell Error instead of rejecting.
    pub(super) fn reinstall_external(&mut self, mut cell: Cell) -> Result<()> {
        let name = cell.name.clone();
        let source = cell
            .source
            .clone()
            .expect("caller checked source is present");
        // Re-derive the kind, so a def edited into `! ...` stops auto-running.
        cell.kind = CellKind::of_source(&source);
        let (deps, parse_err) = parse_deps(cell.kind, def_body(&source), &|d| {
            self.qualify(d, Some(&cell.name))
        });
        cell.deps = deps;
        cell.confirmed = false;
        self.bump_gen(&name);
        if let Some(path) = self.find_cycle(&name, &cell.deps) {
            cell.state = CellState::Error(format!("cycle: {}", path.join(" -> ")));
            self.store.set_state(&cell)?;
            self.put(cell);
            return Ok(());
        }
        cell.state = match parse_err {
            Some(e) => CellState::Error(e),
            None => CellState::Stale,
        };
        let handoff_candidate =
            self.active && cell.kind == CellKind::AgentCall && cell.state == CellState::Stale;
        self.store.set_state(&cell)?;
        self.put(cell);
        // A run handed to this engine arrives with its def; the stale state just
        // written would erase it. Checked after the write: the handing process
        // writes its marker before its state, so a racing handoff is seen here.
        if handoff_candidate {
            if let Some(depth) = self.store.handoff_depth(&name) {
                return self.adopt_handoff(&name, depth);
            }
        }
        self.recompute(vec![name])
    }

    /// Queue a run another process handed over, at the depth it asked for.
    pub(super) fn adopt_handoff(&mut self, name: &str, depth: u32) -> Result<()> {
        self.adopted_depth.insert(name.to_string(), depth);
        self.store.mark_running(name)?;
        if let Some(cell) = self.cells.get_mut(name) {
            cell.state = CellState::Queued;
            self.store.set_state(cell)?;
        }
        if !self.agent_queue.iter().any(|n| n == name) {
            self.agent_queue.push_back(name.to_string());
        }
        self.touched.insert(name.to_string());
        self.log(format!(
            "{name} was handed over by an agent's process; it runs here"
        ));
        self.start_queued()
    }

    /// Non-interactive edit decision: suggest when an agent owns the cell, else pin.
    pub(super) fn hold_external_override(&mut self, mut cell: Cell, written: String) -> Result<()> {
        let name = cell.name.clone();
        if matches!(cell.def_by, Some(Actor::Agent(_))) {
            self.log(format!("external write to {name} held as suggestion"));
            let duplicate = cell
                .pending_suggestions
                .last()
                .is_some_and(|s| s.value == written);
            if !duplicate {
                let prov = Provenance::now(Actor::Human);
                cell.pending_suggestions.push(crate::model::Suggestion {
                    value: written,
                    from: prov.by,
                    note: None,
                    at: prov.at,
                });
            }
            // Restore the computed value; the suggestion rides in meta.
            self.store.update(&cell)?;
            self.put(cell);
            Ok(())
        } else {
            // Pin: the cell becomes a literal, even if the text starts like a def.
            self.log(format!("external write pinned {name}"));
            self.set_literal(&name, &written, Actor::Human)?;
            self.bump_gen(&name);
            self.recompute(vec![name])
        }
    }

    /// Reconcile every cell on disk, for tests that write files directly.
    #[cfg(test)]
    pub(crate) fn reconcile_all_for_test(&mut self) {
        for cell in self.store.list().unwrap() {
            self.reconcile(&cell.name).unwrap();
        }
    }
}
