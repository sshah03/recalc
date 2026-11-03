//! Questions on cells and their answers, from agents and from the person.

use super::*;

impl Engine {
    /// Handle a streaming run's event: an activity update, or a question posted
    /// on the cell. The question is asked as the cell's agent, not the run's MCP
    /// id, so the agent can't answer it itself.
    pub(super) fn stream_event(&mut self, name: String, gen: u64, ev: stream::Event) -> Result<()> {
        if self.run_gen.get(&name) != Some(&gen) {
            return Ok(()); // a run that was stopped or superseded; the reply drops
        }
        let Some(cell) = self.cells.get_mut(&name) else {
            return Ok(());
        };
        match ev {
            stream::Event::Activity(what) => {
                cell.activity = Some(what);
                self.touched.insert(name);
            }
            stream::Event::Ask(ask) => {
                let options = ask
                    .options
                    .into_iter()
                    .map(|(label, preview)| crate::model::DecisionOption {
                        value: label.clone(),
                        label,
                        preview,
                    })
                    .collect();
                let decision = Decision {
                    question: ask.question,
                    options,
                    allow_freetext: ask.freetext,
                    recommended: None,
                    asker: Actor::Agent(format!("{name}'s agent")),
                    posed_at: now_ms(),
                    held: None,
                    from_run: true,
                    answerable_by: None,
                };
                let mut cell = cell.clone();
                self.log(format!("{name}'s agent asks: {}", decision.question));
                cell.decision = Some(decision);
                self.store.update(&cell)?;
                self.put(cell);
                self.touched.insert(name.clone());
                self.pending_asks.insert(name, ask.reply);
                // Its slot is free while it waits: start the next in line.
                self.start_queued()?;
            }
        }
        Ok(())
    }

    /// Answers another recalc process wrote for questions this one's runs
    /// are waiting on.
    pub(super) fn collect_answers(&mut self) -> Result<bool> {
        let names: Vec<String> = self.pending_asks.keys().cloned().collect();
        let mut any = false;
        for name in names {
            let Some(raw) = self.store.take_answer(&name) else {
                continue;
            };
            let Some(answer) = answer_from_json(&raw) else {
                continue;
            };
            if let Some(reply) = self.pending_asks.remove(&name) {
                let _ = reply.send(answer);
            }
            if let Some(cell) = self.cells.get_mut(&name) {
                cell.decision = None;
                self.touched.insert(name);
            }
            any = true;
        }
        Ok(any)
    }

    /// Take down the question a streaming run left on `name`, if any. The
    /// run, if still going, reads the dropped reply as a refusal.
    pub(super) fn drop_stream_ask(&mut self, name: &str) {
        // One a run in another process left (that process gone) goes too.
        let left_behind = self
            .cells
            .get(name)
            .and_then(|c| c.decision.as_ref())
            .is_some_and(|d| d.from_run);
        if self.pending_asks.remove(name).is_some() || left_behind {
            if let Some(cell) = self.cells.get_mut(name) {
                cell.decision = None;
                let _ = self.store.update(cell);
                self.touched.insert(name.to_string());
            }
        }
        if let Some(cell) = self.cells.get_mut(name) {
            if cell.activity.take().is_some() {
                self.touched.insert(name.to_string());
            }
        }
    }

    /// Put a cell into Awaiting with a pending decision.
    pub fn pose_decision(&mut self, name: &str, decision: Decision) -> Result<()> {
        let mut cell = self
            .cells
            .get(name)
            .cloned()
            .unwrap_or_else(|| Cell::new(name));
        self.log_as(
            &decision.asker,
            format!("asks on {name}: {}", decision.question),
        );
        cell.decision = Some(decision);
        cell.state = CellState::Awaiting;
        self.store.update(&cell)?;
        self.put(cell);
        Ok(())
    }

    /// Send a person's answer to the running agent that asked on `name`, and
    /// take the question down.
    pub(super) fn answer_stream_ask(
        &mut self,
        name: &str,
        choice: Choice,
        by: Actor,
    ) -> Result<()> {
        let Some(decision) = self.cells.get(name).and_then(|c| c.decision.clone()) else {
            bail!("no pending decision on {name}");
        };
        let (answer, said) = match choice {
            Choice::Index(i) => {
                let Some(opt) = decision.options.get(i) else {
                    bail!("no option {} on {name}", i + 1);
                };
                (stream::Answer::Option(i), opt.label.clone())
            }
            Choice::Text(_) if !decision.allow_freetext => {
                bail!("{name} does not allow a freetext answer")
            }
            Choice::Text(t) => (stream::Answer::Text(t.clone()), t),
        };
        match self.pending_asks.remove(name) {
            Some(reply) => {
                let _ = reply.send(answer);
            }
            // The run is in another recalc process, which reads the answer from the store.
            None => self.store.write_answer(name, &answer_json(&answer))?,
        }
        let mut cell = self.cells[name].clone();
        cell.decision = None;
        self.store.update(&cell)?;
        self.put(cell);
        self.touched.insert(name.to_string());
        self.log_as(&by, format!("answered {name}'s agent: {said}"));
        Ok(())
    }

    /// A person typed a plain value over a computed cell: hold it and ask
    /// suggest / pin / override / cancel. Suggest needs an agent-owned def.
    pub fn edit_computed(&mut self, name: &str, typed: String) -> Result<()> {
        let Some(cell) = self.cells.get(name) else {
            bail!("no such cell: {name}");
        };
        if cell.source.is_none() {
            bail!("{name} is a literal; set it directly");
        }
        let mut options = Vec::new();
        if let Some(Actor::Agent(id)) = &cell.def_by {
            options.push(crate::model::DecisionOption {
                label: format!("Suggest to agent {id}"),
                value: "suggest".into(),
                preview: None,
            });
        }
        let agent_owned = !options.is_empty();
        for (label, value) in [
            ("Pin", "pin"),
            ("Override once", "override"),
            ("Cancel", "cancel"),
        ] {
            options.push(crate::model::DecisionOption {
                label: label.into(),
                value: value.into(),
                preview: None,
            });
        }
        let prov = Provenance::now(Actor::System);
        self.pose_decision(
            name,
            Decision {
                question: format!("you edited a computed cell ({name})"),
                options,
                // Freetext is a suggestion, so it exists only when Suggest does.
                allow_freetext: agent_owned,
                recommended: None,
                asker: Actor::System,
                posed_at: prov.at,
                held: Some(typed),
                from_run: false,
                answerable_by: None,
            },
        )
    }

    /// Resolve a pending decision. An agent's choice becomes the cell's value;
    /// a system edit decision picks suggest/pin/override/cancel, and freetext
    /// means suggest with this note.
    pub fn resolve_decision(&mut self, name: &str, choice: Choice, by: Actor) -> Result<()> {
        let from_run = self
            .cells
            .get(name)
            .and_then(|c| c.decision.as_ref())
            .is_some_and(|d| d.from_run);
        if from_run {
            return self.answer_stream_ask(name, choice, by);
        }
        let Some(cell) = self.cells.get(name) else {
            bail!("no such cell: {name}");
        };
        let Some(decision) = cell.decision.clone() else {
            bail!("no pending decision on {name}");
        };

        if decision.asker == Actor::System {
            let held = decision.held.clone().unwrap_or_default();
            let (action, note) = match &choice {
                Choice::Index(i) => {
                    let Some(opt) = decision.options.get(*i) else {
                        bail!("no option {i} on {name}");
                    };
                    (opt.value.as_str(), None)
                }
                Choice::Text(_) if !decision.allow_freetext => {
                    bail!("{name} does not allow a freetext answer")
                }
                Choice::Text(note) => ("suggest", Some(note.clone())),
            };
            self.log_as(&by, format!("on {name}: {action}"));
            match action {
                "suggest" => {
                    let mut cell = self.cells.get(name).cloned().expect("checked above");
                    let prov = Provenance::now(by);
                    cell.pending_suggestions.push(Suggestion {
                        value: held,
                        from: prov.by,
                        note,
                        at: prov.at,
                    });
                    cell.decision = None;
                    cell.state = CellState::Ok;
                    self.store.update(&cell)?;
                    self.put(cell);
                    Ok(())
                }
                "pin" => {
                    self.set_literal(name, &held, by)?;
                    self.bump_gen(name);
                    self.recompute(vec![name.to_string()])
                }
                "override" => {
                    // Use the value now; the source reclaims the cell on its
                    // next recompute.
                    let mut cell = self.cells.get(name).cloned().expect("checked above");
                    cell.value = held;
                    cell.decision = None;
                    cell.state = CellState::Ok;
                    cell.provenance = Provenance::now(by);
                    self.store.update(&cell)?;
                    self.put(cell);
                    self.bump_gen(name);
                    let dependents = self.dependents_of(name);
                    self.recompute(dependents)
                }
                _ => {
                    // cancel: discard the edit, revert.
                    let mut cell = self.cells.get(name).cloned().expect("checked above");
                    cell.decision = None;
                    cell.state = CellState::Ok;
                    self.store.set_state(&cell)?;
                    self.put(cell);
                    Ok(())
                }
            }
        } else {
            // Agent-posed: the answer becomes the cell's value.
            let value = match &choice {
                Choice::Index(i) => {
                    let Some(opt) = decision.options.get(*i) else {
                        bail!("no option {i} on {name}");
                    };
                    opt.value.clone()
                }
                Choice::Text(t) if decision.allow_freetext => t.clone(),
                Choice::Text(_) => bail!("{name} does not allow a freetext answer"),
            };
            self.log_as(&by, format!("answered {name}: {value}"));
            self.set_literal(name, &value, by)?;
            self.bump_gen(name);
            self.recompute(vec![name.to_string()])
        }
    }
}

/// An answer as `.recalc/answers/<cell>` holds it.
pub(super) fn answer_json(a: &stream::Answer) -> String {
    match a {
        stream::Answer::Option(i) => serde_json::json!({ "option": i }),
        stream::Answer::Text(t) => serde_json::json!({ "text": t }),
    }
    .to_string()
}

pub(super) fn answer_from_json(raw: &str) -> Option<stream::Answer> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    match (v["option"].as_u64(), v["text"].as_str()) {
        (Some(i), _) => Some(stream::Answer::Option(i as usize)),
        (None, Some(t)) => Some(stream::Answer::Text(t.to_string())),
        _ => None,
    }
}
