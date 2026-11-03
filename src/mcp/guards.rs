//! What an agent may change: verified cells, its own inputs, other agents' answers, the shell.

use super::*;

impl Server {
    /// Verified by a person: an agent's value write becomes a suggestion, and
    /// its mark, shape and citation are refused.
    pub(super) fn verified(&self, name: &str) -> bool {
        self.engine
            .cell(name)
            .and_then(|c| c.mark.as_ref())
            .is_some_and(|m| m.kind == MarkKind::Verified)
    }

    pub(super) fn refuse_if_verified(
        &self,
        name: &str,
        what: &str,
    ) -> std::result::Result<(), String> {
        if self.verified(name) {
            return Err(format!(
                "{name} is verified by a person; an agent can't change its {what}. Say why in a set_cell note (it lands as a suggestion), or ask_cell"
            ));
        }
        Ok(())
    }

    /// Why a cell agent may not change `target` directly: it is one of the
    /// agent's own inputs or another agent's answer. Values written there are
    /// held as suggestions; marks, citations and answer types are refused.
    pub(super) fn outside_its_cell(&self, target: &str) -> Option<String> {
        if !self.cell_agent {
            return None;
        }
        let own = self.default_id.as_deref()?;
        if target == own {
            return None;
        }
        if self.engine.cell(own)?.deps.iter().any(|d| d == target) {
            return Some(format!("{target} is one of {own}'s inputs"));
        }
        if self
            .engine
            .cell(target)
            .is_some_and(|c| c.kind == CellKind::AgentCall)
        {
            return Some(format!("{target} is another agent's answer"));
        }
        None
    }

    pub(super) fn refuse_outside(
        &self,
        targets: &[String],
        what: &str,
    ) -> std::result::Result<(), String> {
        match targets.iter().find_map(|t| self.outside_its_cell(t)) {
            Some(why) => Err(format!(
                "{why}; an agent computing one cell can't change its {what}. Answer in your own cell, or flag your own cell with a note for a person"
            )),
            None => Ok(()),
        }
    }

    /// A cell agent's write or trigger that would reach the shell, refused.
    pub(super) fn shell_refusal(&self, tool: &str, args: &Value) -> Option<String> {
        if !self.cell_agent {
            return None;
        }
        let shell = |kind: CellKind| matches!(kind, CellKind::Command | CellKind::Action);
        // `$1,200` into a shaped cell is an answer, not a command.
        let installs_shell = |name: &str, content: &str| {
            let answer = self
                .engine
                .cell(name)
                .and_then(|c| c.answer_type.as_ref())
                .is_some_and(|sh| sh.check(content).is_ok());
            !answer && shell(CellKind::of_source(content))
        };
        let str_of = |v: &Value| v.as_str().unwrap_or_default().to_string();
        let name = str_of(&args["name"]);
        let writes_shell = match tool {
            "set_cell" => installs_shell(&name, &str_of(&args["content"])),
            "append_cell" => installs_shell(&name, &str_of(&args["value"])),
            "set_range" if args["cells"].is_object() => args["cells"]
                .as_object()
                .into_iter()
                .flatten()
                .any(|(n, v)| installs_shell(n, &str_of(v))),
            "set_range" => {
                let from = str_of(&args["from"]);
                let (sheet, col0, row0) = cell_coord(&from).unwrap_or((None, 0, 0));
                args["values"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .enumerate()
                    .any(|(r, row)| {
                        row.as_array()
                            .into_iter()
                            .flatten()
                            .enumerate()
                            .any(|(c, v)| {
                                installs_shell(
                                    &on_sheet(sheet, &a1_name(col0 + c as u32, row0 + r as u32)),
                                    &str_of(v),
                                )
                            })
                    })
            }
            "trigger_cell" => self.engine.cell(&name).is_some_and(|c| shell(c.kind)),
            "run_range" => range_names(args)
                .iter()
                .any(|n| self.engine.cell(n).is_some_and(|c| shell(c.kind))),
            _ => false,
        };
        writes_shell.then(|| {
            "an agent computing a cell can't write or run $ commands or ! actions; put what you'd run in your answer or a note for a person".to_string()
        })
    }

    /// The entries a cell's agent may write directly, after holding the rest
    /// (its inputs, other agents' answers) as suggestions: `(write, held)`.
    pub(super) fn hold_outside(
        &mut self,
        entries: Vec<(String, String)>,
        by: &Actor,
    ) -> std::result::Result<(Entries, Vec<String>), String> {
        let mut keep = Vec::new();
        let mut held = Vec::new();
        for (name, content) in entries {
            match self.outside_its_cell(&name) {
                Some(why) => {
                    self.engine
                        .suggest(&name, &content, by.clone(), None, &why)
                        .map_err(|e| e.to_string())?;
                    held.push(name);
                }
                None => keep.push((name, content)),
            }
        }
        Ok((keep, held))
    }
}
