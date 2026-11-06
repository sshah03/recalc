//! Agent commands, live sessions and the driver.

use super::*;

impl Engine {
    /// The shell command an `@agent(...)` source resolves to now: prompt refs
    /// interpolated, grid context appended.
    ///
    /// Each headless run gets its own MCP config (`recalc mcp --cell <name>`), so
    /// its writes carry the cell's name.
    pub(super) fn agent_command(&self, name: &str, source: &str) -> Result<String, String> {
        let mcp = self.store.mcp_cell_config_path(name);
        let depth = self.depth + 1;
        self.agent_command_with(name, source, false, &|call, setup, task, _| {
            agent::command(call, setup, task, &mcp.to_string_lossy(), name, depth)
        })
    }

    /// The command a live session of this agent cell runs in its PTY, under
    /// conversation id `session`.
    pub fn session_info(&self, name: &str, session: &str) -> Option<(String, PathBuf)> {
        let cell = self.cells.get(name)?;
        if cell.kind != CellKind::AgentCall {
            return None;
        }
        let cmd = self
            .agent_command_with(
                name,
                cell.source.as_deref()?,
                true,
                &|call, setup, task, mcp| agent::session_command(call, setup, task, mcp, session),
            )
            .ok()?;
        Some((as_agent(name, &cmd), self.launch_dir.clone()))
    }

    /// `session_info`, with the recap of what the board kept in front of the
    /// task: for a new conversation in place of one this computer doesn't have.
    pub fn session_info_recapped(&self, name: &str, session: &str) -> Option<(String, PathBuf)> {
        let recap = self.recap(name)?;
        let cell = self.cells.get(name)?;
        if cell.kind != CellKind::AgentCall {
            return None;
        }
        let cmd = self
            .agent_command_with(
                name,
                cell.source.as_deref()?,
                true,
                &|call, setup, task, mcp| {
                    agent::session_command(call, setup, &format!("{recap}\n\n{task}"), mcp, session)
                },
            )
            .ok()?;
        Some((as_agent(name, &cmd), self.launch_dir.clone()))
    }

    /// What the board kept about agent cell `name`, said to a new
    /// conversation that replaces one from another computer: its answer and
    /// who wrote it, a person's mark, its citation, its earlier answers. Built
    /// from the board, so nothing extra is saved or shared for it.
    pub fn recap(&self, name: &str) -> Option<String> {
        let cell = self.cells.get(name)?;
        let clip = |s: &str, n: usize| {
            let s = s.trim();
            match s.char_indices().nth(n) {
                Some((i, _)) => format!("{}…", &s[..i]),
                None => s.to_string(),
            }
        };
        let who = |a: &Actor| match a {
            Actor::Human => "a person".to_string(),
            Actor::Agent(id) if id == name => "you".to_string(),
            Actor::Agent(id) => format!("agent {id}"),
            _ => "recalc".to_string(),
        };
        let mut lines = Vec::new();
        if cell.value.trim().is_empty() {
            lines.push("- It has no answer yet.".to_string());
        } else {
            lines.push(format!(
                "- Its answer now, written by {}: \"{}\"",
                who(&cell.provenance.by),
                clip(&cell.value, 400)
            ));
        }
        if let Some(mark) = &cell.mark {
            let did = match mark.kind {
                crate::model::MarkKind::Verified => "verified it",
                crate::model::MarkKind::Flagged => "flagged it",
            };
            let note = mark
                .note
                .as_deref()
                .map(|n| format!(": \"{}\"", clip(n, 300)))
                .unwrap_or_default();
            lines.push(format!("- {} {did}{note}", sentence_case(&who(&mark.by))));
        }
        if let Some(cite) = &cell.cite {
            lines.push(format!(
                "- It cites {}: \"{}\"",
                cite.file,
                clip(&cite.quote, 200)
            ));
        }
        let earlier: Vec<String> = self
            .history(name, 4)
            .unwrap_or_default()
            .into_iter()
            .rev()
            .map(|(_, v)| v)
            .filter(|v| !v.trim().is_empty() && *v != cell.value)
            .take(3)
            .map(|v| format!("\"{}\"", clip(&v, 120)))
            .collect();
        if !earlier.is_empty() {
            lines.push(format!(
                "- Its earlier answers, newest first: {}",
                earlier.join(", ")
            ));
        }
        Some(format!(
            "You worked on this cell before, on another computer. That conversation isn't on this one, so this is a new conversation. What the board kept:\n{}\nPick up from there; get_cell and history have the rest.",
            lines.join("\n")
        ))
    }

    /// Say how many agents deep this engine runs (see `depth`).
    pub fn set_depth(&mut self, depth: u32) {
        self.depth = depth;
    }

    /// Hand agent runs to the active engine when one is alive elsewhere
    /// (see `hand_off_runs`).
    pub fn set_hand_off_runs(&mut self, on: bool) {
        self.hand_off_runs = on;
    }

    /// The agent an agent cell calls (`claude` in `@claude(...)`).
    pub fn agent_of(&self, name: &str) -> Option<String> {
        let source = self.cells.get(name)?.source.as_deref()?;
        agent::parse(def_body(source)).ok().map(|c| c.agent)
    }

    /// The program whose conversations this agent cell's are (`claude`,
    /// `codex` or `pi`), a custom template's included. None when recalc can't tell.
    pub fn program_of(&self, name: &str) -> Option<&'static str> {
        agent::program_of(&self.agent_of(name)?, self.setup())
    }

    /// Whether `agent`'s runs can take `model=`: the builtins can, a custom
    /// template only where it says `{model}`.
    pub fn agent_takes_model(&self, agent: &str) -> bool {
        self.config
            .agents
            .get(agent)
            .is_none_or(|t| t.contains("{model}"))
    }

    /// Whether this agent cell's live session runs under an id recalc
    /// chooses (see `agent::session_takes_id`).
    pub fn session_takes_id(&self, name: &str) -> bool {
        self.cells
            .get(name)
            .and_then(|c| c.source.as_deref())
            .and_then(|s| agent::parse(def_body(s)).ok())
            .is_some_and(|call| agent::session_takes_id(&call.agent, self.setup()))
    }

    /// The command that resumes this cell's last conversation in a terminal.
    pub fn resume_info(&self, name: &str) -> Result<(String, PathBuf), String> {
        let cell = self.cells.get(name).ok_or("no such cell")?;
        if cell.kind != CellKind::AgentCall {
            return Err(format!("{name} is not an agent cell"));
        }
        let session = cell
            .session_id
            .as_deref()
            .ok_or("no conversation to continue: run the cell or open a session first")?;
        let call = agent::parse(def_body(cell.source.as_deref().unwrap_or_default()))?;
        let dirs = self.input_dirs(name);
        let cmd = agent::resume_command(
            &call,
            agent::Setup {
                dirs: &dirs,
                ..self.setup()
            },
            session,
            &self.store.mcp_config_path().to_string_lossy(),
        )?;
        // As the cell, like its sessions: its writes carry the cell's name, and
        // `recalc hook stop` finds its row.
        Ok((as_agent(name, &cmd), self.launch_dir.clone()))
    }

    /// The driver's command: the configured driver agent, wired to the board as
    /// `driver`, under conversation id `session`.
    pub fn driver_info(&self, session: &str) -> Result<(String, PathBuf), String> {
        let mcp = self
            .store
            .ensure_mcp_config_for("driver")
            .map_err(|e| e.to_string())?;
        let role = self
            .store
            .prompt_override("driver")
            .unwrap_or_else(|| agent::DRIVER_PROMPT.to_string());
        let prompt = self
            .store
            .ensure_prompt_file("driver", &role)
            .map_err(|e| e.to_string())?;
        let cmd = agent::driver_command(
            &self.config.driver,
            self.setup(),
            &mcp.to_string_lossy(),
            &role,
            &prompt.to_string_lossy(),
            session,
        )?;
        Ok((as_agent("driver", &cmd), self.launch_dir.clone()))
    }

    /// The command that continues the driver's conversation `session`, once
    /// its terminal is gone. Err for a driver recalc can't resume.
    pub fn driver_resume_info(&self, session: &str) -> Result<(String, PathBuf), String> {
        let mcp = self
            .store
            .ensure_mcp_config_for("driver")
            .map_err(|e| e.to_string())?;
        let role = self
            .store
            .prompt_override("driver")
            .unwrap_or_else(|| agent::DRIVER_PROMPT.to_string());
        let prompt = self
            .store
            .ensure_prompt_file("driver", &role)
            .map_err(|e| e.to_string())?;
        let cmd = agent::driver_resume_command(
            &self.config.driver,
            self.setup(),
            &mcp.to_string_lossy(),
            &role,
            &prompt.to_string_lossy(),
            session,
        )?;
        Ok((as_agent("driver", &cmd), self.launch_dir.clone()))
    }

    /// The program the driver's conversations belong to (`claude`, `codex` or `pi`),
    /// a custom template's included. None when recalc can't tell.
    pub fn driver_program(&self) -> Option<&'static str> {
        agent::program_of(&self.config.driver, self.setup())
    }

    /// Whether the driver runs under an id recalc chooses (not Codex's).
    pub fn driver_takes_id(&self) -> bool {
        agent::driver_takes_id(&self.config.driver, self.setup())
    }

    /// A board's own version of a built-in prompt (`driver`, `instructions`).
    pub fn prompt_override(&self, name: &str) -> Option<String> {
        self.store.prompt_override(name)
    }

    /// Record the conversation id an agent cell's session or run runs under.
    pub fn set_session_id(&mut self, name: &str, id: Option<String>) -> Result<()> {
        let mut cell = self
            .current(name)
            .ok_or_else(|| anyhow::anyhow!("no such cell {name}"))?;
        cell.session_id = id;
        self.store.update(&cell)?;
        self.put(cell);
        Ok(())
    }

    /// Who can take the driver's seat: the builtins, then the board's own
    /// `[agents]` templates that can reach the board (and the one seated).
    pub fn driver_choices(&self) -> Vec<String> {
        let mut custom: Vec<String> = self
            .config
            .agents
            .keys()
            .filter(|k| !matches!(k.as_str(), "claude" | "codex" | "pi"))
            // One with no way to reach the board can't drive it (a sandboxed
            // eval contestant, a local model).
            .filter(|k| agent::has_board_tools(k, self.setup()) || **k == self.config.driver)
            .cloned()
            .collect();
        custom.sort();
        ["claude", "codex", "pi"]
            .into_iter()
            .map(str::to_string)
            .chain(custom)
            .collect()
    }

    /// Seat a different agent as the driver the next time it starts.
    pub fn set_driver(&mut self, agent: &str, by: Actor) -> Result<()> {
        if !self.driver_choices().iter().any(|c| c == agent) {
            bail!("{agent} isn't an agent this board knows; add it under [agents] in .recalc/config.toml");
        }
        // Another agent can't resume this one's conversation.
        if self.config.driver != agent {
            self.store.clear_driver_session()?;
        }
        self.store.set_driver(agent)?;
        self.config_stamp = self.store.config_stamp();
        self.config = self.store.config()?;
        self.log_as(&by, format!("seated {agent} as the driver"));
        Ok(())
    }

    /// Every conversation id the board holds: its cells' and the driver's.
    pub fn session_ids(&self) -> Vec<String> {
        self.cells
            .values()
            .filter_map(|c| c.session_id.clone())
            .chain(self.driver_session())
            .collect()
    }

    /// What the Driver's finished runs used: dollars at API prices, tokens.
    pub fn driver_usage(&self) -> (f64, u64) {
        let usage = self.store.driver_usage();
        (
            usage.runs.iter().map(|r| r.cost_usd).sum(),
            usage.runs.iter().map(|r| r.tokens).sum(),
        )
    }

    /// Count each Driver run as it ends, from Claude Code's own record of the
    /// last session in this board's folder. Looked at every few seconds, and
    /// read only when that file changed. A Codex driver reports nothing here.
    pub(super) fn collect_driver_usage(&mut self) {
        if self.driver_checked.elapsed() < std::time::Duration::from_secs(3) {
            return;
        }
        self.driver_checked = std::time::Instant::now();
        let Some(state) = self.claude_state.clone() else {
            return;
        };
        let mut usage = self.store.driver_usage();
        let mut changed = false;
        // Every conversation the Driver has had: one replaced by Start new
        // ends after the board has moved on to the next.
        if let Some(id) = self.driver_session() {
            if !usage.seen.contains(&id) {
                usage.seen.push(id);
                changed = true;
            }
        }
        let stamp = std::fs::metadata(&state)
            .ok()
            .and_then(|m| Some((m.modified().ok()?, m.len())));
        if stamp != self.claude_state_stamp {
            self.claude_state_stamp = stamp;
            if let Some(run) = crate::session::claude_last_run(&state, &self.launch_dir) {
                let counted = usage
                    .runs
                    .iter()
                    .any(|r| r.session == run.session && r.started == run.started);
                if usage.seen.contains(&run.session) && !counted {
                    self.log(format!(
                        "the Driver's session ended; it used ${:.2} at API prices",
                        run.cost_usd
                    ));
                    usage.runs.push(run);
                    changed = true;
                    self.news = true;
                }
            }
        }
        if changed {
            let _ = self.store.set_driver_usage(&usage);
        }
    }

    /// The driver's conversation id for this board, minted once and kept.
    pub fn driver_session(&self) -> Option<String> {
        self.store.driver_session()
    }

    pub fn set_driver_session(&self, id: &str) -> Result<()> {
        self.store.set_driver_session(id)
    }

    /// Ask the window on this board to do something (see `Store::request`).
    pub fn request(&self, kind: &str, name: &str) -> Result<()> {
        self.store.request(kind, name)
    }

    pub fn take_requests(&self) -> Vec<(String, String)> {
        self.store.take_requests()
    }

    pub(super) fn setup(&self) -> agent::Setup<'_> {
        agent::Setup {
            agents: &self.config.agents,
            paths: &self.config.paths,
            models: &self.config.models,
            resume: &self.config.resume,
            dirs: &[],
        }
    }

    /// Folders outside the board that hold documents this cell's inputs name
    /// (a document grid over a folder picked elsewhere). Its agent may read
    /// there as it reads the board, and nowhere else outside it.
    pub(super) fn input_dirs(&self, name: &str) -> Vec<String> {
        let Some(cell) = self.cells.get(name) else {
            return Vec::new();
        };
        let root = self
            .launch_dir
            .canonicalize()
            .unwrap_or_else(|_| self.launch_dir.clone());
        let mut dirs = std::collections::BTreeSet::new();
        for dep in &cell.deps {
            let Some(input) = self.cells.get(dep) else {
                continue;
            };
            // A file-group cell lists several: "a.md, b.md".
            for part in input.value.split([',', '\n']).map(str::trim) {
                if !part.contains('/') || part.len() > 1024 {
                    continue;
                }
                let Ok(path) = self.launch_dir.join(part).canonicalize() else {
                    continue;
                };
                if path.is_file() && !path.starts_with(&root) {
                    if let Some(parent) = path.parent() {
                        dirs.insert(parent.to_string_lossy().into_owned());
                    }
                }
            }
        }
        dirs.into_iter().collect()
    }

    pub(super) fn agent_command_with(
        &self,
        name: &str,
        source: &str,
        live: bool,
        build: agent::Build<'_>,
    ) -> Result<String, String> {
        let call = agent::parse(def_body(source))?;
        let task = self.agent_task(name, &call, live);
        let mcp = self.store.mcp_config_path();
        let dirs = self.input_dirs(name);
        let setup = agent::Setup {
            dirs: &dirs,
            ..self.setup()
        };
        build(&call, setup, &task, &mcp.to_string_lossy())
    }

    /// The prompt an agent cell's run (or `live` session) is given: its call,
    /// refs filled in, the grid context, and the footer about the cell.
    pub(super) fn agent_task(&self, name: &str, call: &agent::Call, live: bool) -> String {
        let mut task =
            agent::task_text(call, &|dep, form| self.interp_lookup_from(name, dep, form));
        // Tell the agent the answer type, unless the prompt already does.
        let tools = agent::has_board_tools(&call.agent, self.setup());
        if let Some(answer_type) = self.cells.get(name).and_then(|c| c.answer_type.as_ref()) {
            let sentence = answer_type.sentence();
            if answer_type.kind != AnswerKind::Text && !task.contains(&sentence) {
                let cite = if tools {
                    "; cite with cite_cell instead"
                } else {
                    ""
                };
                task.push_str(&format!(
                    "\n\n{sentence} Print only that: no explanation{cite}."
                ));
            }
        }
        // A flagged answer: the next run is told what was flagged and why, so
        // it checks again instead of repeating it.
        if let Some(cell) = self.cells.get(name) {
            if let Some(mark) = cell
                .mark
                .as_ref()
                .filter(|m| m.kind == crate::model::MarkKind::Flagged)
            {
                let clip = |s: &str, n: usize| {
                    let s = s.trim();
                    match s.char_indices().nth(n) {
                        Some((i, _)) => format!("{}…", &s[..i]),
                        None => s.to_string(),
                    }
                };
                let who = match &mark.by {
                    Actor::Agent(id) => format!("Agent {id}"),
                    _ => "A reviewer".to_string(),
                };
                let why = mark
                    .note
                    .as_deref()
                    .filter(|n| !n.trim().is_empty())
                    .map(|n| format!(", saying: \"{}\"", clip(n, 500)))
                    .unwrap_or_default();
                task.push_str(&format!(
                    "\n\n{who} flagged the previous answer, \"{}\"{why}. Take the note as the reviewer's guidance on what the question means, look again, and answer.",
                    clip(&cell.value, 300)
                ));
            }
        }
        task.push_str(&agent::footer(name, &call.agent, live, tools));
        task
    }
}

/// `cmd` run as agent `id`, so `recalc` from its shell writes as that agent
/// (see `cli::me`). Exported first so it covers every command in a template.
pub(super) fn as_agent(id: &str, cmd: &str) -> String {
    format!(
        "export RECALC_AGENT={}; {cmd}",
        crate::interp::shell_quote(id)
    )
}

fn sentence_case(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}
