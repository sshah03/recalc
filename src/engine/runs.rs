//! Running `$`, `!` and `@` cells: spawning, the agent queue, usage limits and budgets.

use super::*;

impl Engine {
    /// Mark runs over whose process has gone; otherwise the cell refuses every
    /// trigger as already running.
    ///
    /// Active engine only: a passive engine's snapshot trails, so a run that just
    /// ended looks orphaned there.
    pub(super) fn reset_orphaned_runs(&mut self) -> Result<()> {
        if !self.active {
            return Ok(());
        }
        let orphans: Vec<String> = self
            .cells
            .values()
            .filter(|c| matches!(c.state, CellState::Computing | CellState::Queued))
            .filter(|c| self.store.run_orphaned(&c.name))
            .map(|c| c.name.clone())
            .collect::<Vec<_>>();
        // Run adopted cells in reading order.
        let mut orphans = orphans;
        orphans.sort_by_key(|n| {
            crate::model::a1_coord(n)
                .map(|(c, r)| (r, c))
                .unwrap_or((u32::MAX, 0))
        });
        let mut adopted = false;
        for name in orphans {
            let cell = self.cells.get_mut(&name).expect("collected above");
            // Queued but never started: run it here. Computing means the run died with
            // its process.
            if cell.kind == CellKind::AgentCall && cell.state == CellState::Queued {
                if let Some(depth) = self.store.handoff_depth(&name) {
                    self.adopted_depth.insert(name.clone(), depth);
                }
                self.store.mark_running(&name)?;
                if !self.agent_queue.iter().any(|n| *n == name) {
                    self.agent_queue.push_back(name.clone());
                }
                self.touched.insert(name.clone());
                self.log(format!(
                    "{name} was queued by a process that has gone; it runs from here"
                ));
                adopted = true;
                continue;
            }
            cell.state = CellState::Stale;
            // A question its run asked has no one left to read the answer.
            if cell.decision.as_ref().is_some_and(|d| d.from_run) {
                cell.decision = None;
            }
            self.store.set_state(cell)?;
            self.store.clear_running(&name);
            self.touched.insert(name.clone());
            self.log(format!("{name}'s run ended with the process that ran it"));
        }
        if adopted {
            self.start_queued()?;
        }
        Ok(())
    }

    /// Agent runs in flight and waiting for a slot.
    pub fn agent_load(&self) -> (usize, usize) {
        (self.agents_running, self.agent_queue.len())
    }

    /// Agent runs holding a slot; a run waiting on a person's answer doesn't.
    pub(super) fn agents_working(&self) -> usize {
        self.agents_running.saturating_sub(self.pending_asks.len())
    }

    /// Until when the agent queue is held after a rate limit (Unix ms).
    pub fn agents_paused_until(&self) -> Option<u64> {
        self.agents_paused_until
    }

    /// Start the queue again once a usage-limit hold has run out.
    pub(super) fn lift_expired_hold(&mut self) -> Result<bool> {
        if self.agents_paused_until.is_none() || self.agents_paused() {
            return Ok(false);
        }
        self.agents_paused_until = None;
        self.log("the agent usage-limit hold ended; queued agent cells start again".into());
        self.start_queued()?;
        Ok(true)
    }

    /// What this board's agent runs have reported spending, in dollars.
    pub fn spent(&self) -> f64 {
        self.cells
            .values()
            .filter_map(|c| c.usage.as_ref().and_then(|u| u.cost_usd))
            .sum()
    }

    /// The board has spent its `budget_usd`: queued agent cells wait.
    pub fn over_budget(&self) -> bool {
        self.config.budget_usd.is_some_and(|b| self.spent() >= b)
    }

    /// Set or clear the board's spending limit; raising it restarts the queue.
    pub fn set_budget(&mut self, dollars: Option<f64>, by: Actor) -> Result<()> {
        if dollars.is_some_and(|d| d.is_nan() || d < 0.0 || !d.is_finite()) {
            bail!("a budget is a number of dollars, 0 or more");
        }
        self.store.set_budget(dollars)?;
        self.config_stamp = self.store.config_stamp();
        self.config = self.store.config()?;
        self.budget_noted = false;
        self.log_as(
            &by,
            match dollars {
                Some(d) => format!("set the agent budget to ${d:.2}"),
                None => "removed the agent budget".into(),
            },
        );
        self.start_queued()
    }

    pub(super) fn agents_paused(&self) -> bool {
        self.agents_paused_until
            .is_some_and(|until| now_ms() < until)
    }

    /// Hold the queue until the limit resets, or for ten minutes if the agent
    /// gave no reset time.
    pub(super) fn pause_agents(&mut self, resets_at: Option<u64>) {
        let until = resets_at
            .map(|secs| secs * 1000)
            .filter(|&ms| ms > now_ms())
            .unwrap_or_else(|| now_ms() + 10 * 60 * 1000);
        self.agents_paused_until = Some(until);
        let queued = self.agent_queue.len();
        self.log(format!(
            "an agent hit its usage limit; {queued} queued agent cell{} held for {} min (run one to try sooner)",
            if queued == 1 { "" } else { "s" },
            (until - now_ms()).div_ceil(60_000)
        ));
    }

    /// The per-run agent timeout from the board's config, in ms.
    pub fn agent_timeout_ms(&self) -> u64 {
        self.config.agent_timeout_ms
    }

    /// Whether a run this engine started is still going or waiting to.
    pub fn runs_in_progress(&self) -> bool {
        self.agents_running > 0 || !self.agent_queue.is_empty() || self.in_flight > 0
    }

    /// Requeue a run that hit the usage limit, keeping the cell's last value.
    /// Returns false if the run was superseded.
    pub(super) fn requeue_limited(&mut self, done: &RunDone) -> Result<bool> {
        if self.run_gen.get(&done.name) != Some(&done.gen) {
            return Ok(false);
        }
        self.stoppers.remove(&done.name);
        self.store.clear_running(&done.name);
        self.drop_stream_ask(&done.name);
        let Some(cell) = self.cells.get_mut(&done.name) else {
            return Ok(false);
        };
        if cell.state == CellState::Awaiting {
            return Ok(false);
        }
        cell.state = CellState::Queued;
        self.store.set_state(cell)?;
        self.touched.insert(done.name.clone());
        self.agent_queue.retain(|n| *n != done.name);
        self.agent_queue.push_front(done.name.clone());
        self.log(format!(
            "{} hit the agent's usage limit; it waits in the queue for the hold to lift",
            done.name
        ));
        Ok(true)
    }

    pub(super) fn apply_run(&mut self, done: RunDone) -> Result<()> {
        let agent = done.agent;
        if agent {
            self.agents_running -= 1;
            // Decide on a hold before the next queued cell starts.
            if let Ok(out) = &done.result {
                if let Err(e) = agent::parse_output(out.stdout.trim()) {
                    if e.rate_limited {
                        self.pause_agents(e.resets_at);
                        if self.requeue_limited(&done)? {
                            return Ok(());
                        }
                    }
                }
            }
        }
        let applied = self.apply_result(done);
        // After the answer lands, so a cell queued behind it sees it.
        if agent {
            self.start_queued()?;
        }
        applied
    }

    pub(super) fn apply_result(&mut self, done: RunDone) -> Result<()> {
        if self.run_gen.get(&done.name) != Some(&done.gen) {
            // Superseded by a newer run, a stop or a write. With nothing else
            // running the cell, its run marker goes too.
            let idle = self
                .cells
                .get(&done.name)
                .is_some_and(|c| !matches!(c.state, CellState::Computing | CellState::Queued));
            if idle {
                self.stoppers.remove(&done.name);
                self.store.clear_running(&done.name);
            }
            return Ok(());
        }
        self.stoppers.remove(&done.name);
        self.store.clear_running(&done.name);
        self.drop_stream_ask(&done.name);
        // The agent may have cited this cell through its own MCP server, which the
        // mirror may not reflect yet. The run cleared the citation at start, so
        // what's on disk is this run's.
        let cited = self
            .store
            .get(&done.name)
            .ok()
            .flatten()
            .and_then(|c| c.cite);
        let mut loose_note = None;
        let cell = self.cells.get_mut(&done.name).expect("ran cells exist");
        if cell.state == CellState::Awaiting {
            return Ok(()); // a decision landed mid-run; don't clobber it
        }
        if cited.is_some() {
            cell.cite = cited;
        }
        match done.result {
            Ok(out) => {
                let mut stdout = out.stdout.trim().to_string();
                let mut stderr = out.stderr;
                let mut misfit = None;
                if cell.kind == CellKind::AgentCall {
                    // Builtin templates print a machine-readable result with the answer and cost.
                    match agent::parse_output(&stdout) {
                        Ok(parsed) => {
                            stdout = parsed.text;
                            cell.usage = parsed.usage;
                            // An agent that doesn't report its model (Codex) is
                            // recorded with the one its call asked for.
                            if let Some(usage) = cell.usage.as_mut().filter(|u| u.model.is_none()) {
                                usage.model = cell
                                    .source
                                    .as_deref()
                                    .and_then(|s| agent::parse(super::def_body(s)).ok())
                                    .and_then(|call| {
                                        agent::resolved_model(
                                            &call,
                                            agent::Setup {
                                                agents: &self.config.agents,
                                                paths: &self.config.paths,
                                                models: &self.config.models,
                                                resume: &self.config.resume,
                                                dirs: &[],
                                            },
                                        )
                                    });
                            }
                            if parsed.session_id.is_some() {
                                cell.session_id = parsed.session_id;
                            }
                        }
                        Err(e) => {
                            misfit = Some(format!("agent error: {e}"));
                            cell.usage = None;
                        }
                    }
                    // Check the answer against the cell's answer type.
                    if misfit.is_none() {
                        if let Some(answer_type) = &cell.answer_type {
                            match answer_type.check_printed(&stdout) {
                                Ok(canonical) => stdout = canonical,
                                Err(e) if answer_type.strict => {
                                    if !stderr.is_empty() {
                                        stderr.push('\n');
                                    }
                                    stderr.push_str("answer as printed: ");
                                    stderr.push_str(&stdout);
                                    misfit =
                                        Some(format!("answer doesn't fit its answer type: {e}"));
                                }
                                Err(e) => {
                                    loose_note = Some(format!(
                                        "{} answered outside its answer type: {e}",
                                        done.name
                                    ))
                                }
                            }
                        }
                    }
                }
                // An agent that exits non-zero with no output keeps its last value and
                // errors. Exit 127 is command not found.
                cell.state = if let Some(msg) = misfit {
                    CellState::Error(msg)
                } else if cell.kind == CellKind::AgentCall
                    && out.exit_code != 0
                    && stdout.is_empty()
                {
                    CellState::Error(if out.exit_code == 127 {
                        let agent = cell
                            .source
                            .as_deref()
                            .and_then(|s| agent::parse(def_body(s)).ok())
                            .map(|c| c.agent)
                            .unwrap_or_else(|| "agent".into());
                        if self.config.agents.contains_key(&agent) {
                            format!("{agent}: command not found; check its template under [agents] in .recalc/config.toml")
                        } else {
                            agent::missing_program(&agent)
                        }
                    } else {
                        format!("agent exited {}; stderr is in the console", out.exit_code)
                    })
                } else {
                    cell.value = stdout;
                    CellState::Ok
                };
                cell.exit_code = Some(out.exit_code);
                cell.stderr = stderr;
                cell.duration_ms = Some(out.duration_ms);
            }
            // Failed to launch or timed out; keep the last value.
            Err(msg) => cell.state = CellState::Error(msg),
        }
        cell.provenance = Provenance::now(Actor::Engine);
        self.store.update(cell)?;
        self.touched.insert(done.name.clone());
        if let Some(note) = loose_note {
            self.log(note);
        }
        let entry = format!(
            "engine ran {} -> {}",
            done.name,
            self.cells[&done.name].state.word()
        );
        self.log(entry);
        let dependents = self.dependents_of(&done.name);
        self.recompute(dependents)?;
        self.sync_rows(&done.name)
    }

    /// Run a `!` action or `@` agent cell, or force-refresh a `$` cell; the only
    /// way `!`/`@` cells run. A human trigger also confirms a quarantined `$` cell.
    pub fn trigger(&mut self, name: &str, by: Actor) -> Result<()> {
        // While the queue is held, a person running a queued agent cell takes it
        // out of line and runs it now.
        let held = self.agents_paused_until.is_some() || self.over_budget();
        let take_from_queue = held
            && matches!(by, Actor::Human | Actor::Cli)
            && self
                .cells
                .get(name)
                .is_some_and(|c| c.kind == CellKind::AgentCall && c.state == CellState::Queued);
        if take_from_queue {
            self.agent_queue.retain(|n| n != name);
        }
        let Some(cell) = self.cells.get_mut(name) else {
            bail!("no such cell: {name}");
        };
        match cell.kind {
            CellKind::Command | CellKind::Action | CellKind::AgentCall => {
                // Refuse a second trigger on a long run rather than silently superseding
                // it. `$` cells stay re-triggerable: that is force-refresh.
                if cell.kind != CellKind::Command
                    && !take_from_queue
                    && matches!(cell.state, CellState::Computing | CellState::Queued)
                {
                    let doing = if cell.state == CellState::Queued {
                        "queued"
                    } else {
                        "running"
                    };
                    bail!("{name} is already {doing}; its result will land in the cell");
                }
                if cell.kind != CellKind::AgentCall {
                    let source = cell
                        .source
                        .as_deref()
                        .expect("computed cells have a source");
                    if let Some(e) = source_error(cell.kind, def_body(source)) {
                        bail!("{name}: {e}"); // don't run a cell installed as Error
                    }
                }
                // An agent can't be first to run an `!` action or quarantined `$` that an
                // agent wrote, or an injected agent could write and run its own command.
                // A person's run confirms it until the source changes.
                if matches!(by, Actor::Agent(_))
                    && cell.kind == CellKind::AgentCall
                    && self.depth >= MAX_AGENT_DEPTH
                {
                    bail!(
                        "{name}: agents here are already {} deep, started by other agents; one this deep can't start another. Put the question in your answer for the agent above you.",
                        self.depth
                    );
                }
                let agent_wrote = matches!(cell.def_by, Some(Actor::Agent(_)));
                let unvetted = cell.kind == CellKind::Action
                    || (cell.kind == CellKind::Command && cell.state == CellState::Quarantined);
                if matches!(by, Actor::Agent(_)) && agent_wrote && unvetted && !cell.confirmed {
                    bail!(
                        "{name} runs a command an agent wrote; a person runs it the first time (Run in the window, or `recalc trigger {name}`)"
                    );
                }
                if matches!(by, Actor::Human | Actor::Cli) {
                    cell.confirmed = true;
                    self.touched.insert(name.to_string());
                }
                if matches!(by, Actor::Human | Actor::Cli)
                    && cell.kind == CellKind::AgentCall
                    && !self.in_bulk
                {
                    self.budget_exempt.insert(name.to_string());
                }
                if matches!(by, Actor::Human | Actor::Cli)
                    && cell.kind == CellKind::AgentCall
                    && self.agents_paused_until.take().is_some()
                {
                    self.log(format!(
                        "{name} was run by hand; the usage-limit hold is lifted"
                    ));
                }
                self.log_as(&by, format!("ran {name}"));
                self.spawn_run(name)
            }
            _ => bail!("{name} is not a command, action, or agent cell"),
        }
    }

    /// Stop a running or queued run: kill its process group, discard its result,
    /// and leave the cell Stale. Returns whether there was anything to stop.
    pub fn stop(&mut self, name: &str, by: Actor) -> Result<bool> {
        let Some(cell) = self.cells.get(name) else {
            bail!("no such cell: {name}");
        };
        match cell.state {
            CellState::Computing => {
                if let Some(tx) = self.stoppers.remove(name) {
                    let _ = tx.send(());
                }
                // The run's result, when it lands, is from a dead generation.
                self.bump_gen(name);
                self.drop_stream_ask(name);
            }
            CellState::Queued => self.agent_queue.retain(|n| n != name),
            _ => return Ok(false),
        }
        let cell = self.cells.get_mut(name).expect("checked above");
        cell.state = CellState::Stale;
        self.store.set_state(cell)?;
        self.store.clear_running(name);
        self.touched.insert(name.to_string());
        self.log_as(&by, format!("stopped {name}"));
        Ok(true)
    }

    /// Stop everything running or queued. Returns how many were stopped.
    pub fn stop_all(&mut self, by: Actor) -> Result<usize> {
        let busy: Vec<String> = self
            .cells
            .values()
            .filter(|c| matches!(c.state, CellState::Computing | CellState::Queued))
            .map(|c| c.name.clone())
            .collect();
        let mut n = 0;
        for name in busy {
            if self.stop(&name, by.clone())? {
                n += 1;
            }
        }
        Ok(n)
    }

    /// Run every runnable cell in `names` that isn't running, queued or
    /// verified, and unless `all`, only those out of date. Literals and empties
    /// are skipped; cells the danger scan flags are returned for confirmation.
    pub fn trigger_many(&mut self, names: &[String], by: Actor, all: bool) -> RunSelection {
        let mut out = RunSelection::default();
        self.in_bulk = true;
        let order = self.in_dependency_order(names);
        for name in &order {
            let Some(cell) = self.cells.get(name) else {
                continue;
            };
            let runnable = matches!(
                cell.kind,
                CellKind::Command | CellKind::Action | CellKind::AgentCall
            );
            if !runnable
                || cell.verified()
                || matches!(cell.state, CellState::Computing | CellState::Queued)
            {
                continue;
            }
            if !all && cell.state == CellState::Ok {
                out.up_to_date.push(name.clone());
                continue;
            }
            if self.needs_confirm(name) {
                out.need_confirm.push(name.clone());
                continue;
            }
            match self.trigger(name, by.clone()) {
                Ok(()) => out.ran.push(name.clone()),
                Err(e) => self.log(format!("{name}: {e}")),
            }
        }
        self.in_bulk = false;
        out
    }

    pub(super) fn bump_gen(&mut self, name: &str) {
        if let Some(gen) = self.run_gen.get_mut(name) {
            *gen += 1;
        }
    }

    /// Should a frontend ask before explicitly running this cell?
    /// Quarantined cells and actions with destructive-looking sources do.
    pub fn needs_confirm(&self, name: &str) -> bool {
        self.cells.get(name).is_some_and(|c| {
            c.state == CellState::Quarantined
                || c.source
                    .as_deref()
                    .is_some_and(|s| danger::scan(def_body(s), &self.config.danger_tokens))
        })
    }

    /// What a command cell would run now, after interpolation; what the console
    /// shows and `spawn_run` executes.
    pub fn console_info(&self, name: &str) -> Option<Console> {
        let cell = self.cells.get(name)?;
        let source = cell.source.as_deref()?;
        if cell.kind == CellKind::AgentCall {
            // On error there is no command; triggering surfaces the message.
            let command = self.agent_command(name, source).ok()?;
            return Some(Console {
                command,
                cwd: self.launch_dir.clone(),
                watch: None,
                every: None,
            });
        }
        let (body, ann) = split_annotations(def_body(source));
        let command = interp::resolve(body, &|dep, form| self.interp_lookup_from(name, dep, form));
        let cwd = match ann.cwd.or(self.config.cwd.as_deref()) {
            Some(p) => self.launch_dir.join(p),
            None => self.launch_dir.clone(),
        };
        Some(Console {
            command,
            cwd,
            watch: ann.watch.map(str::to_string),
            every: ann.every_ms().ok().flatten(),
        })
    }

    /// Cells whose source doesn't parse, with why. Meant for a freshly loaded
    /// board: a template or demo that has any is broken.
    pub fn source_problems(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = self
            .cells
            .keys()
            .filter_map(|n| Some((n.clone(), self.source_problem(n)?)))
            .collect();
        out.sort();
        out
    }

    /// Why this cell's source can't run as written, or None. Not whether its
    /// last value was an error.
    pub fn source_problem(&self, name: &str) -> Option<String> {
        let c = self.cells.get(name)?;
        let source = c.source.as_deref()?;
        match c.kind {
            CellKind::Expr => expr::parse(def_body(source)).err(),
            CellKind::AgentCall => agent::parse(def_body(source)).err(),
            CellKind::Command | CellKind::Action => source_error(c.kind, def_body(source)),
            CellKind::Literal => None,
        }
    }

    /// Resolve and launch a `$`/`!`/`@` cell's command as a background task.
    pub(super) fn spawn_run(&mut self, name: &str) -> Result<()> {
        if self.hand_off_runs
            && !self.active
            && self.cells[name].kind == CellKind::AgentCall
            && self.store.active_engine_elsewhere()
        {
            // The window runs it, since this process may exit first; the result
            // reaches this engine through the store.
            self.store.mark_handoff(name, self.depth + 1)?;
            let cell = self.cells.get_mut(name).expect("looked up by the caller");
            cell.state = CellState::Queued;
            self.store.set_state(cell)?;
            self.touched.insert(name.to_string());
            return Ok(());
        }
        // The prompt, when the agent takes it over stdin (`stream::run`).
        let mut stream_task: Option<String> = None;
        let mut protocol = agent::Protocol::Plain;
        let (resolved, cwd) = match self.cells[name].kind {
            CellKind::AgentCall => {
                let source = self.cells[name]
                    .source
                    .clone()
                    .expect("agent cells have a source");
                let depth = self
                    .adopted_depth
                    .get(name)
                    .copied()
                    .unwrap_or(self.depth + 1);
                self.store.ensure_mcp_config_for_cell(name, depth)?;
                let cmd = self
                    .agent_command(name, &source)
                    .map_err(|e| anyhow::anyhow!("{name}: {e}"))?;
                let call =
                    agent::parse(def_body(&source)).map_err(|e| anyhow::anyhow!("{name}: {e}"))?;
                protocol = agent::protocol(&call.agent, self.setup());
                if protocol != agent::Protocol::Plain {
                    stream_task = Some(self.agent_task(name, &call, false));
                }
                (cmd, self.launch_dir.clone())
            }
            _ => {
                let c = self
                    .console_info(name)
                    .expect("command cells have a source");
                (c.command, c.cwd)
            }
        };

        let is_agent = self.cells[name].kind == CellKind::AgentCall;
        if is_agent
            && (self.agents_working() >= self.config.max_agents.max(1)
                || self.agents_paused()
                || self.reads_a_run(name)
                || (self.over_budget() && !self.budget_exempt.remove(name)))
        {
            // No slot, or an input still to come: queue it. The command is resolved
            // again at start, so inputs edited meanwhile apply.
            self.store.mark_running(name)?;
            let cell = self.cells.get_mut(name).expect("looked up above");
            cell.state = CellState::Queued;
            self.store.set_state(cell)?;
            self.touched.insert(name.to_string());
            if !self.agent_queue.iter().any(|n| n == name) {
                self.agent_queue.push_back(name.to_string());
            }
            return Ok(());
        }
        // Starting now: the MCP config above already carries its depth.
        self.adopted_depth.remove(name);

        let gen = {
            let g = self.run_gen.entry(name.to_string()).or_insert(0);
            *g += 1;
            *g
        };
        self.store.mark_running(name)?;
        let cell = self.cells.get_mut(name).expect("looked up above");
        cell.state = CellState::Computing;
        // A citation backs one answer; the new run brings its own or none.
        cell.cite = None;
        self.store.set_state(cell)?;
        self.touched.insert(name.to_string());

        let tx = self.runs_tx.clone();
        let name = name.to_string();
        let shell = self.config.shell.clone();
        let mut env = self.config.env.clone();
        if is_agent {
            // `recalc` from the agent's shell writes as the agent. A headless run's
            // answer is its cell's value, so `recalc hook stop` lets it stop.
            env.insert("RECALC_AGENT".into(), name.clone());
            env.insert("RECALC_HEADLESS".into(), "1".into());
        }
        let timeout = std::time::Duration::from_millis(if is_agent {
            self.config.agent_timeout_ms
        } else {
            self.config.timeout_ms
        });
        self.in_flight += 1;
        if is_agent {
            self.agents_running += 1;
        }
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
        self.stoppers.insert(name.clone(), stop_tx);
        let groups = self.groups.clone();
        let events = self.stream_tx.clone();
        tokio::spawn(async move {
            let result = match stream_task {
                Some(task) if protocol == agent::Protocol::CodexEvents => {
                    let who = name.clone();
                    let on_event = move |ev| {
                        let _ = events.send((who.clone(), gen, ev));
                    };
                    stream::run_codex(
                        &shell, &resolved, &cwd, &env, timeout, stop_rx, &groups, &task, on_event,
                    )
                    .await
                }
                Some(task) if protocol == agent::Protocol::PiEvents => {
                    let who = name.clone();
                    let on_event = move |ev| {
                        let _ = events.send((who.clone(), gen, ev));
                    };
                    stream::run_pi(
                        &shell, &resolved, &cwd, &env, timeout, stop_rx, &groups, &task, on_event,
                    )
                    .await
                }
                Some(task) => {
                    let who = name.clone();
                    let on_event = move |ev| {
                        let _ = events.send((who.clone(), gen, ev));
                    };
                    stream::run(
                        &shell, &resolved, &cwd, &env, timeout, stop_rx, &groups, &task, on_event,
                    )
                    .await
                }
                None => exec::run(&shell, &resolved, &cwd, &env, timeout, stop_rx, &groups).await,
            };
            // Send can only fail if the engine is gone; nothing to do then.
            let _ = tx.send(RunDone {
                name,
                gen,
                agent: is_agent,
                result,
            });
        });
        Ok(())
    }

    /// Give freed agent slots to the queue, in order, dropping cells no longer
    /// Queued.
    pub(super) fn start_queued(&mut self) -> Result<()> {
        if self.agents_paused() {
            return Ok(());
        }
        if self.over_budget() && !self.agent_queue.is_empty() {
            if !std::mem::replace(&mut self.budget_noted, true) {
                let queued = self.agent_queue.len();
                self.log(format!(
                    "the board has spent its ${:.2} agent budget; {queued} queued agent cell{} wait (raise the budget, or run one yourself)",
                    self.config.budget_usd.unwrap_or_default(),
                    if queued == 1 { "" } else { "s" },
                ));
            }
            return Ok(());
        }
        // A cell that reads a queued or running cell waits for it, rather than
        // answering from blanks.
        let mut held = Vec::new();
        while self.agents_working() < self.config.max_agents.max(1) {
            let Some(name) = self.agent_queue.pop_front() else {
                break;
            };
            let still_waiting = self
                .cells
                .get(&name)
                .is_some_and(|c| c.kind == CellKind::AgentCall && c.state == CellState::Queued);
            if !still_waiting {
                continue;
            }
            if self.reads_a_run(&name) {
                held.push(name);
                continue;
            }
            if let Err(e) = self.spawn_run(&name) {
                let cell = self.cells.get_mut(&name).expect("checked above");
                cell.state = CellState::Error(e.to_string());
                self.store.set_state(cell)?;
                self.touched.insert(name.clone());
            }
        }
        for name in held.into_iter().rev() {
            self.agent_queue.push_front(name);
        }
        Ok(())
    }

    /// Does `name` depend on a cell that's queued or running, directly or
    /// through formulas over it?
    pub(super) fn reads_a_run(&self, name: &str) -> bool {
        let mut seen = HashSet::new();
        let mut todo: Vec<&str> = self
            .cells
            .get(name)
            .map_or(vec![], |c| c.deps.iter().map(String::as_str).collect());
        while let Some(dep) = todo.pop() {
            if !seen.insert(dep) {
                continue;
            }
            let Some(c) = self.cells.get(dep) else {
                continue;
            };
            if matches!(c.state, CellState::Computing | CellState::Queued) {
                return true;
            }
            if c.kind == CellKind::Expr {
                todo.extend(c.deps.iter().map(String::as_str));
            }
        }
        false
    }
}

/// Trailing `# cwd:`, `# watch: <glob> [<glob>…]` and `# every: <ms>` annotations on a
/// command body. Any other `#` is a shell comment and stays.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Annotations<'a> {
    pub(super) cwd: Option<&'a str>,
    pub(super) watch: Option<&'a str>,
    pub(super) every: Option<&'a str>,
    pub(super) rows: Option<&'a str>,
}

impl Annotations<'_> {
    /// Parsed `# every:` period. Err when present but not a positive integer.
    pub(super) fn every_ms(&self) -> Result<Option<u64>, String> {
        match self.every {
            None => Ok(None),
            Some(s) => match s.parse::<u64>() {
                Ok(ms) if ms > 0 => Ok(Some(ms)),
                _ => Err(format!("every: {s:?} is not a period in milliseconds")),
            },
        }
    }

    /// Why this cell can't have these annotations. Only `$` cells re-run by
    /// themselves; a watched `!` would fire an action nobody triggered.
    pub(super) fn error(&self, kind: CellKind) -> Option<String> {
        if kind == CellKind::Action && (self.watch.is_some() || self.every.is_some()) {
            return Some("watch/every: only $ cells re-run automatically".into());
        }
        if let Err(e) = self.every_ms() {
            return Some(e);
        }
        if let Some(spec) = self.rows {
            if kind != CellKind::Command {
                return Some("rows: only a $ cell's output becomes rows".into());
            }
            if let Err(e) = super::rows::parse_rows_spec(spec) {
                return Some(e);
            }
        }
        // Several globs are separated by spaces: `# watch: src/** tests/*.py`.
        for g in self.watch.into_iter().flat_map(str::split_whitespace) {
            if g.starts_with('/') {
                return Some("watch: globs are relative to the board directory".into());
            }
            if let Err(e) = globset::Glob::new(g) {
                return Some(format!("watch: {e}"));
            }
        }
        None
    }
}

/// Why this command source can't run (bad annotations or repo refs), if it
/// can't. Checked at install, on trigger, and before reactive runs.
pub(super) fn source_error(kind: CellKind, body: &str) -> Option<String> {
    let (cmd, ann) = split_annotations(body);
    ann.error(kind).or_else(|| interp::repo_ref_error(cmd))
}

/// Split trailing annotations off a command body, right to left; the first
/// non-annotation `#` (or the rightmost duplicate key) ends the scan.
pub(super) fn split_annotations(body: &str) -> (&str, Annotations<'_>) {
    let mut cmd = body;
    let mut ann = Annotations::default();
    loop {
        let Some((head, tail)) = cmd.rsplit_once('#') else {
            return (cmd, ann);
        };
        let Some((key, value)) = tail.split_once(':') else {
            return (cmd, ann);
        };
        let slot = match key.trim() {
            "cwd" => &mut ann.cwd,
            "watch" => &mut ann.watch,
            "every" => &mut ann.every,
            "rows" => &mut ann.rows,
            _ => return (cmd, ann),
        };
        let value = value.trim();
        if value.is_empty() || value.contains('\n') || slot.is_some() {
            return (cmd, ann);
        }
        *slot = Some(value);
        cmd = head.trim_end();
    }
}
