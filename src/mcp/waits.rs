//! Tools that block: await_cell, trigger_cell, ask_cell, ask_agent.

use super::*;

impl Server {
    pub(super) fn await_cell(&mut self, conn: u64, id: Value, args: &Value) -> Vec<Outgoing> {
        let name = args["name"].as_str().unwrap_or_default().to_string();
        let predicate = args["predicate"].as_str().map(str::to_string);
        let timeout = args["timeout_ms"]
            .as_u64()
            .unwrap_or(DEFAULT_BLOCK_MS)
            .min(MAX_BLOCK_MS);
        let initial = self.value_of(&name);
        if let Some(p) = &predicate {
            // A bare word is a cell reference; refuse one naming no cell. A braced
            // `{goal}` may name a cell not created yet.
            if let Some(word) = bare_words(p)
                .into_iter()
                .find(|w| self.engine.cell(w).is_none())
            {
                return vec![self.tool_out(
                    conn,
                    id,
                    Err(format!(
                        "predicate {p:?} compares against a cell named {word}, and there is no such cell; to compare with the word itself, quote it: {}",
                        quote_bare_word(p, &word)
                    )),
                )];
            }
            match expr::predicate(&name, p, &|dep| self.engine.formula_lookup(dep)) {
                Ok(true) => {
                    let out = self.tool_out(
                        conn,
                        id,
                        Ok(json!({"value": initial, "changed": false, "satisfied": true})),
                    );
                    return vec![out];
                }
                Ok(false) => {}
                Err(e) => return vec![self.tool_out(conn, id, Err(e))],
            }
        }
        self.push_waiter(
            conn,
            id,
            name,
            Wait::Await { predicate, initial },
            Duration::from_millis(timeout),
        );
        Vec::new()
    }

    /// Register a blocking waiter. A re-call of the same wait on the same
    /// (conn, cell) replaces the old waiter so abandoned calls leave no
    /// orphans; a different kind of wait keeps its own.
    pub(super) fn push_waiter(
        &mut self,
        conn: u64,
        id: Value,
        name: String,
        wait: Wait,
        block: Duration,
    ) {
        let kind = std::mem::discriminant(&wait);
        self.waiters.retain(|w| {
            !(w.conn == conn && w.name == name && std::mem::discriminant(&w.wait) == kind)
        });
        self.waiters.push(Waiter {
            conn,
            id,
            name,
            wait,
            deadline: Instant::now() + block,
        });
    }

    /// Run a cell and wait at most one block; a longer run is reported still
    /// running, not failed.
    pub(super) fn trigger_and_wait(
        &mut self,
        conn: u64,
        id: Value,
        name: String,
        by: Actor,
    ) -> Vec<Outgoing> {
        if let Err(e) = self.engine.trigger(&name, by) {
            return vec![self.tool_out(conn, id, Err(e.to_string()))];
        }
        let config = self.engine.config();
        let run_ms = match self.engine.cell(&name).map(|c| c.kind) {
            Some(CellKind::AgentCall) => config.agent_timeout_ms,
            _ => config.timeout_ms,
        };
        let block = Duration::from_millis((run_ms + 5_000).min(DEFAULT_BLOCK_MS));
        self.push_waiter(conn, id, name, Wait::Trigger, block);
        Vec::new() // deferred
    }

    /// `ask_agent`: `set_cell` with an `@agent('prompt', {inputs})` source plus
    /// `trigger_cell`, so the handoff is an ordinary cell on the board.
    pub(super) fn ask_agent(
        &mut self,
        conn: u64,
        id: Value,
        args: &Value,
        by: Actor,
    ) -> Vec<Outgoing> {
        let built = (|| {
            let name = args["name"]
                .as_str()
                .filter(|n| !n.is_empty())
                .ok_or("ask_agent needs name: the cell the answer goes in")?;
            let agent = args["agent"].as_str().unwrap_or("claude");
            let prompt = args["prompt"]
                .as_str()
                .filter(|p| !p.trim().is_empty())
                .ok_or("ask_agent needs a prompt")?;
            if !self.engine.driver_choices().iter().any(|a| a == agent) {
                return Err(format!("unknown agent {agent:?}: use claude, codex, or a name under [agents] in .recalc/config.toml"));
            }
            if self.verified(name) {
                return Err(format!(
                    "{name} is verified by a person; ask in an empty cell"
                ));
            }
            if let Some(why) = self.outside_its_cell(name) {
                return Err(format!("{why}; ask in an empty cell"));
            }
            // A prompt ends at its last quote, so either quote round-trips any text.
            let quote = if prompt.contains('\'') && !prompt.contains('"') {
                '"'
            } else {
                '\''
            };
            let mut source = format!("@{agent}({quote}{prompt}{quote}");
            for input in args["inputs"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str())
            {
                source.push_str(&format!(", {{{input}}}"));
            }
            source.push(')');
            Ok::<_, String>((name.to_string(), source))
        })();
        let (name, source) = match built {
            Ok(b) => b,
            Err(e) => return vec![self.tool_out(conn, id, Err(e))],
        };
        let note = Some(format!("asked by {}", String::from(by.clone())));
        if let Err(e) = self.engine.set_noted(&name, &source, by.clone(), note) {
            return vec![self.tool_out(conn, id, Err(e.to_string()))];
        }
        self.forget_if_read(conn, &name);
        self.trigger_and_wait(conn, id, name, by)
    }

    pub(super) fn ask_cell(&mut self, conn: u64, id: Value, args: &Value) -> Vec<Outgoing> {
        let name = args["name"].as_str().unwrap_or_default().to_string();
        let question = args["question"].as_str().unwrap_or_default().to_string();
        let timeout = args["timeout_ms"]
            .as_u64()
            .unwrap_or(DEFAULT_BLOCK_MS)
            .min(MAX_BLOCK_MS);

        match self.engine.cell(&name).and_then(|c| c.decision.as_ref()) {
            // Re-asking the same question re-attaches to the pending
            // decision; the agent's long-poll loop depends on this.
            Some(d) if d.question == question => {}
            Some(d) => {
                let msg = format!(
                    "cell {name} has a different pending decision: {}",
                    d.question
                );
                return vec![self.tool_out(conn, id, Err(msg))];
            }
            None => {
                // The answer becomes the cell's value, so asking on a computed cell would
                // discard its result.
                if let Some(src) = self.engine.cell(&name).and_then(|c| c.source.clone()) {
                    let msg = format!(
                        "{name} is computed ({}); its answer would replace what it computed. Ask in a new cell, e.g. {name}_decision",
                        src.chars().take(40).collect::<String>()
                    );
                    return vec![self.tool_out(conn, id, Err(msg))];
                }
                let options = decision_options(&args["options"]);
                // Never picked on timeout: the question stands.
                let recommended = args["recommended"]
                    .as_u64()
                    .map(|n| n as usize)
                    .filter(|i| *i < options.len());
                let decision = Decision {
                    question,
                    options,
                    allow_freetext: args["allow_freetext"].as_bool().unwrap_or(true),
                    recommended,
                    asker: self.agent(conn),
                    posed_at: Provenance::now(Actor::System).at,
                    held: None,
                    from_run: false,
                    answerable_by: args["for"].as_str().map(str::to_string),
                };
                if let Err(e) = self.engine.pose_decision(&name, decision) {
                    return vec![self.tool_out(conn, id, Err(e.to_string()))];
                }
            }
        }
        self.push_waiter(conn, id, name, Wait::Ask, Duration::from_millis(timeout));
        Vec::new()
    }

    pub(super) fn resolve(
        &mut self,
        args: &Value,
        by: Actor,
    ) -> std::result::Result<Value, String> {
        let name = args["name"].as_str().unwrap_or_default();
        let choice = match &args["choice"] {
            Value::Number(n) => {
                let n = n
                    .as_u64()
                    .ok_or("choice must be a positive option number")?;
                Choice::Index((n as usize).saturating_sub(1))
            }
            Value::String(s) => {
                // A string matching an option picks it; anything else is freetext.
                let opt = self
                    .engine
                    .cell(name)
                    .and_then(|c| c.decision.as_ref())
                    .and_then(|d| {
                        d.options
                            .iter()
                            .position(|o| o.value == *s || o.label == *s)
                    });
                match opt {
                    Some(i) => Choice::Index(i),
                    None => Choice::Text(s.clone()),
                }
            }
            _ => return Err("choice must be an option number or a string".into()),
        };
        // An agent may answer only a question it asked or one named for it with
        // `for`; the rest are for a person.
        let decision = self.engine.cell(name).and_then(|c| c.decision.as_ref());
        let named = match (&by, decision.and_then(|d| d.answerable_by.as_deref())) {
            (Actor::Agent(me), Some(them)) => me == them,
            _ => false,
        };
        if decision.is_some_and(|d| d.asker != by && !named) {
            return Err(format!(
                "the question on {name} is for a person; you can answer only a question you asked, or one asked for you"
            ));
        }
        self.engine
            .resolve_decision(name, choice, by)
            .map(|()| json!({"ok": true}))
            .map_err(|e| e.to_string())
    }

    pub(super) fn check_waiters(&mut self) -> Vec<Outgoing> {
        let now = Instant::now();
        let mut outs = Vec::new();
        let mut remaining = Vec::new();
        for waiter in std::mem::take(&mut self.waiters) {
            let done: Option<std::result::Result<Value, String>> = match &waiter.wait {
                Wait::Await { predicate, initial } => {
                    let value = self.value_of(&waiter.name);
                    let met = match predicate {
                        // The parse was validated when the waiter registered,
                        // so an error here is an eval error: not met yet.
                        Some(p) => {
                            expr::predicate(&waiter.name, p, &|dep| self.engine.formula_lookup(dep))
                                .unwrap_or(false)
                        }
                        None => value != *initial,
                    };
                    if met {
                        self.note_read(waiter.conn, &waiter.name);
                        Some(Ok(json!({
                            "value": value,
                            "changed": value != *initial,
                            "satisfied": true,
                        })))
                    } else if now >= waiter.deadline {
                        Some(Ok(json!({
                            "value": value,
                            "changed": false,
                            "satisfied": false,
                        })))
                    } else {
                        None
                    }
                }
                Wait::Ask => {
                    let cell = self.engine.cell(&waiter.name);
                    match cell {
                        Some(c) if c.decision.is_none() => Some(Ok(json!({
                            "resolved": true,
                            "value": c.value,
                            "resolved_by": String::from(c.provenance.by.clone()),
                        }))),
                        // Timeout returns unresolved; the decision stays pending.
                        _ if now >= waiter.deadline => Some(Ok(json!({"resolved": false}))),
                        _ => None,
                    }
                }
                Wait::Trigger => {
                    let cell = self.engine.cell(&waiter.name);
                    match cell {
                        Some(c)
                            if !matches!(
                                c.state,
                                crate::model::CellState::Computing
                                    | crate::model::CellState::Queued
                            ) =>
                        {
                            let out = json!({
                                "value": c.value,
                                "exit_code": c.exit_code,
                                "stderr": c.stderr,
                                "duration_ms": c.duration_ms,
                            });
                            self.note_read(waiter.conn, &waiter.name);
                            Some(Ok(out))
                        }
                        Some(c) if now >= waiter.deadline => Some(Ok(json!({
                            "running": true,
                            "state": if c.state == crate::model::CellState::Queued { "queued" } else { "computing" },
                            "hint": format!("{} is still running; its result will land in the cell. await_cell on it to keep waiting, in short blocks.", waiter.name),
                        }))),
                        None if now >= waiter.deadline => {
                            Some(Err(format!("{} is gone", waiter.name)))
                        }
                        _ => None,
                    }
                }
            };
            match done {
                Some(result) => outs.push(self.tool_out(waiter.conn, waiter.id.clone(), result)),
                None => remaining.push(waiter),
            }
        }
        self.waiters = remaining;
        outs
    }
}

/// Options arrive as ["yes", "no"] or [{"label": "...", "value": "..."}].
pub(super) fn decision_options(value: &Value) -> Vec<DecisionOption> {
    let Some(items) = value.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| match item {
            Value::String(s) => Some(DecisionOption {
                label: s.clone(),
                value: s.clone(),
                preview: None,
            }),
            Value::Object(_) => Some(DecisionOption {
                label: item["label"].as_str()?.to_string(),
                value: item["value"]
                    .as_str()
                    .unwrap_or(item["label"].as_str()?)
                    .to_string(),
                preview: item["preview"].as_str().map(str::to_string),
            }),
            _ => None,
        })
        .collect()
}

/// The words in a predicate that read as cell references without braces:
/// identifiers outside quotes and braces, not followed by `(` (a function),
/// and not the keywords `true` and `false`.
pub(super) fn bare_words(p: &str) -> Vec<String> {
    let chars: Vec<char> = p.chars().collect();
    let (mut out, mut i) = (Vec::new(), 0);
    while i < chars.len() {
        match chars[i] {
            q @ ('\'' | '"') => {
                i += 1;
                while i < chars.len() && chars[i] != q {
                    i += 1;
                }
                i += 1;
            }
            '{' => {
                while i < chars.len() && chars[i] != '}' {
                    i += 1;
                }
                i += 1;
            }
            c if c.is_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                let call = chars[i..].iter().find(|c| !c.is_whitespace()) == Some(&'(');
                if !call && !matches!(word.to_ascii_lowercase().as_str(), "true" | "false") {
                    out.push(word);
                }
            }
            _ => i += 1,
        }
    }
    out
}

/// `p` with the bare word `word` quoted, for the refusal's suggestion.
pub(super) fn quote_bare_word(p: &str, word: &str) -> String {
    let mut out = String::new();
    let mut rest = p;
    while let Some(at) = rest.find(word) {
        let before = rest[..at].chars().last();
        let after = rest[at + word.len()..].chars().next();
        let edge = |c: Option<char>| !c.is_some_and(|c| c.is_alphanumeric() || "_{'\"".contains(c));
        if edge(before) && edge(after) {
            out.push_str(&rest[..at]);
            out.push_str(&format!("'{word}'"));
            out.push_str(&rest[at + word.len()..]);
            return out;
        }
        out.push_str(&rest[..at + word.len()]);
        rest = &rest[at + word.len()..];
    }
    out.push_str(rest);
    out
}
