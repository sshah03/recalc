//! Write tools and the receipts they return.

use super::*;

impl Server {
    /// The write's receipt: the stored value when a shape canonicalized it, a
    /// note when a lenient shape let a misfit through, `unchanged` for a no-op.
    pub(super) fn write_receipt(&self, name: &str, content: &str, before: &str) -> Value {
        let mut out = json!({"ok": true});
        let Some(cell) = self.engine.cell(name) else {
            return out;
        };
        if let Some(answer_type) = &cell.answer_type {
            if cell.source.is_none() {
                match answer_type.check(content) {
                    Ok(canonical) if canonical != content => {
                        out["written"] = json!(canonical);
                        out["from"] = json!(content);
                    }
                    Err(e) if !answer_type.strict => {
                        out["outside_answer_type"] = json!(e);
                    }
                    _ => {}
                }
            }
        }
        if cell.value == before && cell.source.is_none() {
            out["unchanged"] = json!(true);
        }
        // Report a stored source that can't run now, not when its run never starts.
        if cell.source.is_some() {
            if let Some(e) = self.engine.source_problem(name) {
                out["error"] = json!(e);
                out["hint"] = json!(
                    "the cell is saved but can't run until its source is fixed; set it again"
                );
            }
        }
        out
    }

    /// `set_answer_type`: the shape answers to a cell must take, or clear it.
    pub(super) fn set_answer_type(&mut self, args: &Value) -> std::result::Result<Value, String> {
        let name = args["name"].as_str().unwrap_or_default();
        let answer_type = match args["kind"].as_str() {
            None | Some("") | Some("none") => None,
            Some(kind) => {
                let options = args["options"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|o| o.as_str().map(str::to_string))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let mut answer_type = AnswerType::new(kind.parse()?, &options);
                if let Some(strict) = args["strict"].as_bool() {
                    answer_type.strict = strict;
                }
                if let Some(not_found) = args["not_found"].as_bool() {
                    answer_type.allows_not_found = not_found;
                }
                answer_type.validate()?;
                Some(answer_type)
            }
        };
        // A shape on a document grid's header goes on the agent cells under it.
        let targets: Vec<String> = match args["range"].as_str() {
            Some(r) => {
                let (from, to) = r.split_once(':').ok_or("range is like B2:B7")?;
                crate::model::range_cells(from.trim(), to.trim())
                    .ok_or_else(|| format!("bad range {r:?}"))?
            }
            None => {
                let answers = self.answers_under(name);
                if answers.is_empty() {
                    vec![name.to_string()]
                } else {
                    answers
                }
            }
        };
        for t in &targets {
            self.refuse_if_verified(t, "answer type")?;
            self.engine
                .set_answer_type(t, answer_type.clone())
                .map_err(|e| e.to_string())?;
        }
        let mut out = json!({"ok": true});
        if targets.len() != 1 || targets[0] != name {
            out["applied_to"] = json!(targets);
        }
        Ok(out)
    }

    /// The agent cells under a row-1 header in a document grid, when `name`
    /// is one: a literal in row 1 with `@` cells below it in its column.
    pub(super) fn answers_under(&self, name: &str) -> Vec<String> {
        let Some((sheet, col, 1)) = cell_coord(name) else {
            return Vec::new();
        };
        if self.engine.cell(name).is_some_and(|c| c.source.is_some()) {
            return Vec::new();
        }
        let mut out: Vec<(u32, String)> = self
            .engine
            .all_cells()
            .filter(|c| c.kind == CellKind::AgentCall)
            .filter_map(|c| match cell_coord(&c.name) {
                Some((s2, c2, row)) if s2 == sheet && c2 == col && row > 1 => {
                    Some((row, c.name.clone()))
                }
                _ => None,
            })
            .collect();
        out.sort();
        out.into_iter().map(|(_, n)| n).collect()
    }

    /// `open_session` / `show_cell`: a request to the board's window via the
    /// store's inbox; nothing happens when no window is open.
    pub(super) fn request(
        &mut self,
        kind: &str,
        name: &str,
        agent_only: bool,
    ) -> std::result::Result<Value, String> {
        let cell = self
            .engine
            .cell(name)
            .ok_or_else(|| format!("no such cell: {name}"))?;
        if agent_only && cell.kind != crate::model::CellKind::AgentCall {
            return Err(format!("{name} is not an @ agent cell"));
        }
        // The window can't start a session whose source doesn't parse; refuse here.
        if agent_only {
            if let Some(e) = self.engine.source_problem(name) {
                return Err(format!("{name} can't start: {e}; fix its source first"));
            }
        }
        self.engine.request(kind, name).map_err(|e| e.to_string())?;
        Ok(json!({
            "ok": true,
            "note": "asked the window on this board; if none is open, the request waits for one",
        }))
    }

    /// `cite_cell`: attach the passage behind a cell's value, or clear it.
    pub(super) fn cite_cell(
        &mut self,
        args: &Value,
        by: Actor,
    ) -> std::result::Result<Value, String> {
        let name = args["name"].as_str().unwrap_or_default();
        let cite = if args["file"].is_null() && args["quote"].is_null() {
            None
        } else {
            Some(cite_arg(args, by).ok_or("cite_cell needs file and quote")?)
        };
        self.engine
            .set_cite(name, cite)
            .map_err(|e| e.to_string())?;
        let mut out = json!({"ok": true});
        self.note_missing_quote(name, &mut out);
        Ok(out)
    }

    /// Agent cells holding an answer from a finished run.
    pub(super) fn answered(&self) -> std::collections::HashSet<String> {
        self.engine
            .all_cells()
            .filter(|c| {
                c.kind == CellKind::AgentCall
                    && c.state == crate::model::CellState::Ok
                    && !c.value.is_empty()
            })
            .map(|c| c.name.clone())
            .collect()
    }

    /// Name the answers this write made stale by changing a header or the
    /// shared instructions.
    pub(super) fn note_made_stale(
        &self,
        answered: &std::collections::HashSet<String>,
        out: &mut Value,
    ) {
        let mut stale: Vec<&String> = answered
            .iter()
            .filter(|n| {
                self.engine
                    .cell(n)
                    .is_some_and(|c| c.state == crate::model::CellState::Stale)
            })
            .collect();
        if stale.is_empty() {
            return;
        }
        stale.sort();
        out["made_stale"] = json!(stale);
        out["hint"] = json!("these answers were built from what you just changed; trigger_cell them to answer again, or tell the person they need a re-run");
    }

    /// Warn when the cited passage isn't in the file.
    pub(super) fn note_missing_quote(&self, name: &str, out: &mut Value) {
        let cite = self.engine.cell(name).and_then(|c| c.cite.as_ref());
        if let Some(c) = cite.filter(|c| c.found == Some(false)) {
            out["cite_not_found"] = json!(format!(
                "the quoted passage isn't in {} as written; cite it again with the document's exact words",
                c.file
            ));
        }
    }

    /// `mark_cell`: flag a cell (with a note), or clear a mark. Only a human
    /// verifies; an agent asking to verify is refused.
    pub(super) fn mark_cell(
        &mut self,
        args: &Value,
        by: Actor,
    ) -> std::result::Result<Value, String> {
        let name = args["name"].as_str().unwrap_or_default();
        let mark = match args["mark"].as_str().unwrap_or_default() {
            "flagged" => Some(Mark {
                kind: MarkKind::Flagged,
                note: args["note"].as_str().map(str::to_string),
                by: by.clone(),
                at: Provenance::now(by).at,
            }),
            "none" | "" => None,
            "verified" => return Err("only a human can verify a cell".into()),
            other => return Err(format!("mark must be flagged or none, got {other:?}")),
        };
        self.engine
            .set_mark(name, mark)
            .map(|()| json!({"ok": true}))
            .map_err(|e| e.to_string())
    }

    pub(super) fn set_range(
        &mut self,
        args: &Value,
        by: Actor,
    ) -> std::result::Result<Value, String> {
        if let Some(cells) = args["cells"].as_object() {
            return self.set_named(cells, &args["cites"], by);
        }
        let from = args["from"].as_str().unwrap_or_default();
        let (sheet, col0, row0) = cell_coord(from).ok_or_else(|| {
            if from.is_empty() {
                "set_range needs from and values (an A1 block), or cells: {name: content}".to_string()
            } else {
                format!("{from:?} isn't an A1 cell, so it has no block to fill; to write named cells in one call, pass cells: {{\"{from}\": \"…\", …}}")
            }
        })?;
        let rows = args["values"]
            .as_array()
            .ok_or("values must be an array of arrays")?;
        let mut entries = Vec::new();
        for (r, row) in rows.iter().enumerate() {
            let row = row.as_array().ok_or("values must be an array of arrays")?;
            for (c, content) in row.iter().enumerate() {
                let content = content.as_str().ok_or("cell contents must be strings")?;
                entries.push((
                    on_sheet(sheet, &a1_name(col0 + c as u32, row0 + r as u32)),
                    content.to_string(),
                ));
            }
        }
        let (entries, held) = self.hold_outside(entries, &by)?;
        let written: Vec<String> = entries.iter().map(|(n, _)| n.clone()).collect();
        self.engine
            .set_many(entries, by.clone())
            .map_err(|e| e.to_string())?;
        // `cites` mirrors `values`: a {file, quote, where?} or null per cell.
        let mut missing = Vec::new();
        if let Some(rows) = args["cites"].as_array() {
            for (r, row) in rows.iter().enumerate() {
                for (c, v) in row.as_array().into_iter().flatten().enumerate() {
                    let name = on_sheet(sheet, &a1_name(col0 + c as u32, row0 + r as u32));
                    if let Some(cite) = cite_arg(v, by.clone()).filter(|_| !self.verified(&name)) {
                        self.engine
                            .set_cite(&name, Some(cite))
                            .map_err(|e| e.to_string())?;
                        let lost = self
                            .engine
                            .cell(&name)
                            .and_then(|c| c.cite.as_ref())
                            .is_some_and(|c| c.found == Some(false));
                        if lost {
                            missing.push(name);
                        }
                    }
                }
            }
        }
        let mut out = json!({"ok": true});
        if !missing.is_empty() {
            out["cites_not_found"] = json!(missing);
        }
        self.note_source_errors(&written, &mut out);
        if !held.is_empty() {
            out["held_as_suggestions"] = json!(held);
        }
        Ok(out)
    }

    /// Sources a bulk write stored that can't run, reported as `set_cell` does.
    pub(super) fn note_source_errors(&self, names: &[String], out: &mut Value) {
        let errors: serde_json::Map<String, Value> = names
            .iter()
            .filter_map(|n| Some((n.clone(), json!(self.engine.source_problem(n)?))))
            .collect();
        if !errors.is_empty() {
            out["errors"] = Value::Object(errors);
            out["hint"] = json!(
                "those cells are saved but can't run until their sources are fixed; set them again"
            );
        }
    }

    /// `set_range` with `cells: {name: content}`: any cells, named or A1,
    /// written in one wave, exactly as the block form writes its block.
    pub(super) fn set_named(
        &mut self,
        cells: &serde_json::Map<String, Value>,
        cites: &Value,
        by: Actor,
    ) -> std::result::Result<Value, String> {
        let mut entries = Vec::with_capacity(cells.len());
        for (name, content) in cells {
            let content = content.as_str().ok_or("cell contents must be strings")?;
            entries.push((name.clone(), content.to_string()));
        }
        let (entries, held) = self.hold_outside(entries, &by)?;
        let written: Vec<String> = entries.iter().map(|(n, _)| n.clone()).collect();
        self.engine
            .set_many(entries, by.clone())
            .map_err(|e| e.to_string())?;
        let mut missing = Vec::new();
        for (name, v) in cites.as_object().into_iter().flatten() {
            if let Some(cite) = cite_arg(v, by.clone()).filter(|_| !self.verified(name)) {
                self.engine
                    .set_cite(name, Some(cite))
                    .map_err(|e| e.to_string())?;
                if self
                    .engine
                    .cell(name)
                    .and_then(|c| c.cite.as_ref())
                    .is_some_and(|c| c.found == Some(false))
                {
                    missing.push(name.clone());
                }
            }
        }
        let mut out = json!({"ok": true});
        if !missing.is_empty() {
            out["cites_not_found"] = json!(missing);
        }
        self.note_source_errors(&written, &mut out);
        if !held.is_empty() {
            out["held_as_suggestions"] = json!(held);
        }
        Ok(out)
    }
}

/// A citation from tool arguments: `{file, quote, where?}`. None when the
/// arguments don't carry one.
pub(super) fn cite_arg(v: &Value, by: Actor) -> Option<Cite> {
    let file = v["file"].as_str()?;
    let quote = v["quote"].as_str()?;
    Some(Cite {
        file: file.to_string(),
        quote: quote.to_string(),
        location: v["where"].as_str().map(str::to_string),
        at: Provenance::now(by.clone()).at,
        found: None,
        by,
    })
}

/// `(cell, content)` pairs a bulk write carries.
pub(super) type Entries = Vec<(String, String)>;

/// The cells a set_answer_type call names: `name`, `names`, or an A1 `range`.
pub(super) fn answer_type_targets(args: &Value) -> Vec<String> {
    let mut out: Vec<String> = args["names"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    if let Some(n) = args["name"].as_str() {
        out.push(n.to_string());
    }
    if let Some((a, b)) = args["range"].as_str().and_then(|r| r.split_once(':')) {
        out.extend(crate::model::range_cells(a, b).unwrap_or_default());
    }
    out
}
