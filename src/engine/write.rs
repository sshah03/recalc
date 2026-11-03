//! Writing cells: values, sources, pastes, formats, answer types, citations, marks.

use super::*;

impl Engine {
    /// Delete cells outright; dependents read them as empty. Returns
    /// (name, previous content) undo pairs.
    pub fn delete_many(&mut self, names: &[String], by: Actor) -> Result<Vec<(String, String)>> {
        let mut removed = Vec::new();
        for name in names {
            let Some(prev) = self.content(name) else {
                continue;
            };
            self.log_as(&by, format!("deleted {name}"));
            self.store.delete(name)?;
            self.remove(name);
            self.bump_gen(name);
            removed.push((name.clone(), prev));
        }
        let dependents: Vec<String> = removed
            .iter()
            .flat_map(|(n, _)| self.dependents_of(n))
            .collect();
        self.recompute(dependents)?;
        Ok(removed)
    }

    /// Paste copied cell contents with the top-left at `at`, shifting A1
    /// references by the move as fill does. `None` entries leave the target
    /// alone. Returns undo pairs.
    pub fn paste_sources(
        &mut self,
        at: &str,
        origin: &str,
        block: &[Vec<Option<String>>],
        by: Actor,
    ) -> Result<Vec<(String, String)>> {
        // Copied from any sheet; unqualified references then mean the
        // destination's sheet, as in Excel.
        let (Some((sheet, c0, r0)), Some((_, oc, or))) = (cell_coord(at), cell_coord(origin))
        else {
            bail!("paste needs A1 anchors, got {at:?} from {origin:?}");
        };
        let (dc, dr) = (i64::from(c0) - i64::from(oc), i64::from(r0) - i64::from(or));
        let mut entries = Vec::new();
        for (r, row) in block.iter().enumerate() {
            for (c, content) in row.iter().enumerate() {
                let Some(content) = content else {
                    continue;
                };
                let target = on_sheet(sheet, &a1_name(c0 + c as u32, r0 + r as u32));
                let shifted = if CellKind::of_source(content) == CellKind::Literal {
                    content.clone()
                } else {
                    shift_refs(content, dc, dr)
                };
                entries.push((target, shifted));
            }
        }
        self.write_block(entries, by)
    }

    /// Paste plain text as literals: tab-separated if it has tabs, else CSV.
    /// The clipboard form of `import_csv`.
    pub fn paste_text(&mut self, at: &str, text: &str, by: Actor) -> Result<Vec<(String, String)>> {
        let Some((sheet, c0, r0)) = cell_coord(at) else {
            bail!("paste needs an A1 anchor, got {at:?}");
        };
        let rows: Vec<Vec<String>> = if text.contains('\t') {
            text.lines()
                .map(|l| l.split('\t').map(str::to_string).collect())
                .collect()
        } else {
            parse_csv(text)
        };
        let mut entries = Vec::new();
        for (r, row) in rows.iter().enumerate() {
            for (c, field) in row.iter().enumerate() {
                entries.push((
                    on_sheet(sheet, &a1_name(c0 + c as u32, r0 + r as u32)),
                    field.clone(),
                ));
            }
        }
        self.write_literals(entries, by)
    }

    /// Paste values only (⇧⌘V): every entry lands as a literal. `None` entries
    /// and verified targets are skipped. Returns undo pairs.
    pub fn paste_values(
        &mut self,
        at: &str,
        block: &[Vec<Option<String>>],
        by: Actor,
    ) -> Result<Vec<(String, String)>> {
        let Some((sheet, c0, r0)) = cell_coord(at) else {
            bail!("paste needs an A1 anchor, got {at:?}");
        };
        let mut entries = Vec::new();
        for (r, row) in block.iter().enumerate() {
            for (c, value) in row.iter().enumerate() {
                if let Some(value) = value {
                    entries.push((
                        on_sheet(sheet, &a1_name(c0 + c as u32, r0 + r as u32)),
                        value.clone(),
                    ));
                }
            }
        }
        self.write_literals(entries, by)
    }

    /// Write each (cell, text) as a literal whatever its first character, as
    /// undo needs. Verified cells are skipped. Returns undo pairs.
    pub fn set_literals(
        &mut self,
        entries: Vec<(String, String)>,
        by: Actor,
    ) -> Result<Vec<(String, String)>> {
        self.write_literals(entries, by)
    }

    /// Write (target, content) pairs routed like `set`, skipping verified
    /// targets and unchanged cells; one recompute wave. Undo pairs back.
    pub(super) fn write_block(
        &mut self,
        entries: Vec<(String, String)>,
        by: Actor,
    ) -> Result<Vec<(String, String)>> {
        let mut changed = Vec::new();
        let mut next = Vec::new();
        for (target, content) in entries {
            let prev = self.content(&target).unwrap_or_default();
            if prev == content || self.cells.get(&target).is_some_and(Cell::verified) {
                continue;
            }
            let own = self.own_answer(&target, &content, &by);
            if self.write(&target, &content, by.clone(), None).is_ok() {
                next.extend(self.after_write(&target, own));
                changed.push((target, prev));
            }
        }
        self.recompute(next)?;
        Ok(changed)
    }

    /// Like `write_block`, but every field lands as a literal, so a leading `=`
    /// stays text.
    pub(super) fn write_literals(
        &mut self,
        entries: Vec<(String, String)>,
        by: Actor,
    ) -> Result<Vec<(String, String)>> {
        let mut changed = Vec::new();
        for (target, field) in entries {
            let prev = self.content(&target).unwrap_or_default();
            // A def whose source equals the field must still be flattened.
            let already_literal = self.cells.get(&target).is_none_or(|c| c.source.is_none());
            if (prev == field && already_literal)
                || self.cells.get(&target).is_some_and(Cell::verified)
            {
                continue;
            }
            self.set_literal(&target, &field, by.clone())?;
            self.bump_gen(&target);
            changed.push((target, prev));
        }
        self.recompute(changed.iter().map(|(n, _)| n.clone()).collect())?;
        Ok(changed)
    }

    /// Set a cell from user content: `=`/`$`/`!` installs a source, anything
    /// else writes a literal value. Recomputes dependents either way.
    pub fn set(&mut self, name: &str, content: &str, by: Actor) -> Result<()> {
        self.set_noted(name, content, by, None)
    }

    /// `set` plus an optional writer note recorded in provenance.
    pub fn set_noted(
        &mut self,
        name: &str,
        content: &str,
        by: Actor,
        note: Option<String>,
    ) -> Result<()> {
        let name = &self.canonical(name);
        let own = self.own_answer(name, content, &by);
        self.write(name, content, by, note)?;
        self.note_sheet(name);
        let next = self.after_write(name, own);
        self.recompute(next)
    }

    /// Many writes, one recompute wave. Every entry is attempted; the error
    /// lists the refused cells after the rest have settled.
    pub fn set_many(&mut self, entries: Vec<(String, String)>, by: Actor) -> Result<()> {
        let mut written = Vec::with_capacity(entries.len());
        let mut refused = Vec::new();
        for (name, content) in entries {
            let name = self.canonical(&name);
            let own = self.own_answer(&name, &content, &by);
            let written_ok = self.write(&name, &content, by.clone(), None);
            self.note_sheet(&name);
            match written_ok {
                Ok(()) => written.extend(self.after_write(&name, own)),
                Err(e) => refused.push(e.to_string()),
            }
        }
        self.recompute(written)?;
        if refused.is_empty() {
            Ok(())
        } else {
            bail!("{}", refused.join("; "))
        }
    }

    /// Hold `content` as `by`'s suggestion for `name` without writing it. `why`
    /// goes in the activity log.
    pub fn suggest(
        &mut self,
        name: &str,
        content: &str,
        by: Actor,
        note: Option<String>,
        why: &str,
    ) -> Result<()> {
        let Some(mut cell) = self.current(name) else {
            bail!("no such cell: {name}");
        };
        self.log_as(&by, format!("wrote {name}; held as suggestion ({why})"));
        let prov = Provenance::now(by);
        cell.pending_suggestions.push(Suggestion {
            value: content.to_string(),
            from: prov.by,
            note,
            at: prov.at,
        });
        self.store.update(&cell)?;
        self.put(cell);
        Ok(())
    }

    pub(super) fn write(
        &mut self,
        name: &str,
        content: &str,
        by: Actor,
        note: Option<String>,
    ) -> Result<()> {
        if matches!(by, Actor::Agent(_)) && self.cells.get(name).is_some_and(Cell::verified) {
            return self.suggest(name, content, by, note, "verified");
        }
        // An agent's answer must fit the cell's answer type. Human writes aren't
        // checked, since a person may be correcting the type.
        let content = match (
            &by,
            self.cells.get(name).and_then(|c| c.answer_type.as_ref()),
        ) {
            (Actor::Agent(_), Some(answer_type))
                if CellKind::of_source(content) == CellKind::Literal =>
            {
                match answer_type.check(content) {
                    Ok(canonical) => canonical,
                    Err(e) if answer_type.strict => {
                        bail!("{name}: answer doesn't fit its answer type: {e}")
                    }
                    Err(e) => {
                        self.log(format!("{name} answered outside its answer type: {e}"));
                        content.to_string()
                    }
                }
            }
            _ => content.to_string(),
        };
        let content = content.as_str();
        if self.own_answer(name, content, &by) {
            return self.answer_own(name, content, by, note);
        }
        self.log_as(&by, format!("set {name}"));
        match CellKind::of_source(content) {
            CellKind::Literal => self.set_literal(name, content, by)?,
            _ => self.install(name, content, by)?,
        }
        if let Some(note) = note {
            let cell = self.cells.get_mut(name).expect("just written");
            cell.provenance.note = Some(note);
            self.store.set_state(cell)?;
            self.touched.insert(name.to_string());
        }
        // Supersede any in-flight run so its result can't overwrite this value.
        self.bump_gen(name);
        Ok(())
    }

    /// The agent of an `@` cell answering in its own cell (a live session has
    /// no printed answer to land): its answer becomes the value, as a run's
    /// does, and the person's prompt stays.
    pub(super) fn own_answer(&self, name: &str, content: &str, by: &Actor) -> bool {
        (matches!(by, Actor::Agent(id) if id == name) || self.answering.as_deref() == Some(name))
            && CellKind::of_source(content) == CellKind::Literal
            && self
                .cells
                .get(name)
                .is_some_and(|c| c.kind == CellKind::AgentCall)
    }

    /// Another agent answering an `@` cell for it (the Driver, asked to answer
    /// cells itself): the value lands as the cell's answer, as its own agent's
    /// would, with its prompt kept. Verified cells take it as a suggestion,
    /// and it must fit the cell's answer type.
    pub fn answer_cell(
        &mut self,
        name: &str,
        content: &str,
        by: Actor,
        note: Option<String>,
    ) -> Result<()> {
        let name = self.canonical(name);
        let Some(cell) = self.cells.get(&name) else {
            bail!("no such cell: {name}");
        };
        if cell.kind != CellKind::AgentCall {
            bail!("{name} isn't an agent cell; set_cell writes a value there");
        }
        if CellKind::of_source(content) != CellKind::Literal {
            bail!("an answer is a value: {name} keeps its prompt, so it can't hold a formula, command or action");
        }
        self.answering = Some(name.clone());
        let done = self.set_noted(&name, content, by, note);
        self.answering = None;
        done
    }

    /// What a write to `name` recomputes: the cell itself, or for an agent's
    /// own answer only what reads it (the `@` cell isn't stale for it).
    fn after_write(&self, name: &str, own: bool) -> Vec<String> {
        if own {
            self.dependents_of(name)
        } else {
            vec![name.to_string()]
        }
    }

    /// An agent's answer in its own `@` cell: the value, with the source,
    /// mark and the rest of the cell kept. It's final, as any write is: a run
    /// of the cell in flight (here, or in the engine that owns it, which
    /// adopts this write) is superseded rather than left computing.
    fn answer_own(
        &mut self,
        name: &str,
        value: &str,
        by: Actor,
        note: Option<String>,
    ) -> Result<()> {
        let mut cell = self.current(name).expect("checked by the caller");
        self.log_as(&by, format!("answered {name}"));
        if cell.state == CellState::Queued {
            self.agent_queue.retain(|n| n != name);
        }
        cell.value = value.to_string();
        cell.state = CellState::Ok;
        cell.provenance = Provenance::now(by);
        cell.provenance.note = note;
        self.store.update(&cell)?;
        self.put(cell);
        self.bump_gen(name);
        self.touched.insert(name.to_string());
        Ok(())
    }

    pub(super) fn set_literal(&mut self, name: &str, value: &str, by: Actor) -> Result<()> {
        self.check_case(name)?;
        // Series, mark, format and answer type belong to the cell and survive
        // value writes.
        let (series, mark, format, answer_type) = self
            .cells
            .get(name)
            .map(|c| {
                (
                    c.series,
                    c.mark.clone(),
                    c.format.clone(),
                    c.answer_type.clone(),
                )
            })
            .unwrap_or_default();
        self.store.set_value(name, value, by.clone())?;
        let mut cell = Cell::new(name);
        cell.value = value.to_string();
        cell.series = series;
        cell.mark = mark;
        cell.format = format;
        cell.answer_type = answer_type;
        cell.provenance = Provenance::now(by);
        if cell.series || cell.mark.is_some() || cell.format.is_some() || cell.answer_type.is_some()
        {
            self.store.set_state(&cell)?;
        }
        self.put(cell);
        Ok(())
    }

    /// Set or clear a cell's display format. Formatting an empty cell creates it.
    pub fn set_format(&mut self, name: &str, format: Option<Format>) -> Result<()> {
        let mut cell = match self.current(name) {
            Some(c) => c,
            None => {
                crate::store::check_name(name)?;
                Cell::new(name)
            }
        };
        if let Some(f) = &format {
            f.validate().map_err(|e| anyhow::anyhow!("{name}: {e}"))?;
        }
        cell.format = format.filter(|f| !f.is_plain());
        self.store.update(&cell)?;
        self.put(cell);
        Ok(())
    }

    /// Set or clear the answer type for this cell. Like a format, it survives
    /// value writes and may be set on an empty cell.
    pub fn set_answer_type(&mut self, name: &str, answer_type: Option<AnswerType>) -> Result<()> {
        if let Some(sh) = &answer_type {
            sh.validate().map_err(|e| anyhow::anyhow!("{name}: {e}"))?;
        }
        let mut cell = match self.current(name) {
            Some(c) => c,
            None => {
                crate::store::check_name(name)?;
                Cell::new(name)
            }
        };
        cell.answer_type = answer_type;
        self.store.update(&cell)?;
        self.put(cell);
        Ok(())
    }

    /// Attach or detach the passage a value rests on. The cell must exist.
    /// Cleared when the cell runs again or its value is written without one.
    pub fn set_cite(&mut self, name: &str, mut cite: Option<Cite>) -> Result<()> {
        if let Some(c) = cite.as_mut() {
            c.validate().map_err(|e| anyhow::anyhow!("{name}: {e}"))?;
            c.found = self.quote_found(&c.file, &c.quote);
        }
        let Some(mut cell) = self.current(name) else {
            bail!("no such cell: {name}");
        };
        if let Some(c) = &cite {
            self.log_as(&c.by, format!("cited {} for {name}", c.file));
        }
        cell.cite = cite;
        self.store.set_state(&cell)?;
        self.put(cell);
        self.refresh_meta_readers(name)
    }

    /// Whether `quote` is in board-relative `file`. Text is cached per mtime,
    /// since PDF extraction is slow.
    pub(super) fn quote_found(&mut self, file: &str, quote: &str) -> Option<bool> {
        // Another cell is checked against its value now; a web page isn't fetched.
        if let Some(name) = file.strip_prefix("cell:") {
            let value = self.cells.get(name).map(|c| c.value.as_str()).unwrap_or("");
            return Some(crate::source::find_quote(value, quote));
        }
        if file.starts_with("https://") || file.starts_with("http://") {
            return None;
        }
        let path = self.launch_dir.join(file);
        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok()?;
        let fresh = self
            .source_text
            .get(file)
            .is_some_and(|(at, _)| *at == modified);
        if !fresh {
            let text = crate::source::read(&self.launch_dir, file).ok()?.text;
            self.source_text.insert(file.to_string(), (modified, text));
        }
        let (_, text) = self.source_text.get(file)?;
        Some(crate::source::find_quote(text, quote))
    }

    /// Turn down the pending suggestion with this value, or all of them.
    /// Adopting one is an ordinary write, which clears them all.
    pub fn dismiss_suggestions(
        &mut self,
        name: &str,
        value: Option<&str>,
        by: Actor,
    ) -> Result<()> {
        let Some(mut cell) = self.cells.get(name).cloned() else {
            bail!("no such cell: {name}");
        };
        let before = cell.pending_suggestions.len();
        cell.pending_suggestions
            .retain(|s| value.is_some_and(|v| s.value != v));
        if cell.pending_suggestions.len() == before {
            return Ok(());
        }
        self.store.update(&cell)?;
        self.put(cell);
        self.touched.insert(name.to_string());
        self.log_as(&by, format!("turned down a suggestion on {name}"));
        Ok(())
    }

    /// Set or clear a cell's review mark. Marking an empty cell creates it.
    pub fn set_mark(&mut self, name: &str, mark: Option<Mark>) -> Result<()> {
        let mut cell = match self.current(name) {
            Some(c) => c,
            None => {
                crate::store::check_name(name)?;
                Cell::new(name)
            }
        };
        match &mark {
            Some(m) => self.log_as(
                &m.by,
                format!(
                    "marked {name} {}",
                    match m.kind {
                        crate::model::MarkKind::Verified => "verified",
                        crate::model::MarkKind::Flagged => "flagged",
                    }
                ),
            ),
            None => self.log(format!("mark cleared on {name}")),
        }
        cell.mark = mark;
        self.store.update(&cell)?;
        self.put(cell);
        self.refresh_meta_readers(name)
    }

    /// Recompute formulas that read `name`'s meta (`{B2.mark}`) after it
    /// changed without the value changing.
    pub(super) fn refresh_meta_readers(&mut self, name: &str) -> Result<()> {
        let readers: Vec<String> = self
            .dependents_of(name)
            .into_iter()
            .filter(|d| {
                self.cells.get(d).is_some_and(|c| {
                    c.kind == CellKind::Expr
                        && c.source.as_deref().is_some_and(|src| {
                            expr::parse(def_body(src))
                                .is_ok_and(|ast| expr::meta_reads(&ast).iter().any(|m| m == name))
                        })
                })
            })
            .collect();
        if readers.is_empty() {
            return Ok(());
        }
        self.recompute(readers)
    }

    /// The content that recreates a cell: its source, or a literal's value.
    pub fn content(&self, name: &str) -> Option<String> {
        let cell = self.cells.get(name)?;
        Some(cell.source.clone().unwrap_or_else(|| cell.value.clone()))
    }

    /// Toggle sparkline tracking for a cell (persisted in meta).
    pub fn set_series(&mut self, name: &str, on: bool) -> Result<()> {
        let Some(cell) = self.cells.get_mut(name) else {
            bail!("no such cell: {name}");
        };
        cell.series = on;
        self.touched.insert(name.to_string());
        self.store.set_state(cell)
    }

    pub(super) fn install(&mut self, name: &str, source: &str, by: Actor) -> Result<()> {
        self.check_case(name)?;
        let kind = CellKind::of_source(source);
        let (deps, parse_err) =
            parse_deps(kind, def_body(source), &|d| self.qualify(d, Some(name)));

        if let Some(path) = self.find_cycle(name, &deps) {
            // Reject: the source is not applied; the cell shows the cycle.
            let msg = format!("cycle: {}", path.join(" -> "));
            let mut cell = self
                .cells
                .get(name)
                .cloned()
                .unwrap_or_else(|| Cell::new(name));
            cell.state = CellState::Error(msg.clone());
            // `update`, not `set_state`: the cell may not exist yet.
            self.store.update(&cell)?;
            self.put(cell);
            bail!("refusing {name}: {msg}");
        }

        self.store.set_def(name, source, &deps, by.clone())?;
        let state = match parse_err {
            Some(e) => CellState::Error(e),
            None => CellState::Stale,
        };
        let mut cell = self
            .cells
            .get(name)
            .cloned()
            .unwrap_or_else(|| Cell::new(name));
        cell.kind = kind;
        cell.source = Some(source.to_string());
        cell.deps = deps;
        cell.state = state;
        // A changed source must re-earn quarantine confirmation, and an explicit
        // write acts on pending suggestions.
        cell.confirmed = false;
        cell.pending_suggestions.clear();
        if self.pending_asks.remove(name).is_some() {
            cell.decision = None;
        }
        cell.activity = None;
        cell.usage = None;
        cell.def_by = Some(by.clone());
        cell.provenance = Provenance::now(by);
        // `set_def` wrote the meta as stale; persist an install error too, or a
        // sibling process mirrors the stale copy back over it.
        if matches!(cell.state, CellState::Error(_)) {
            self.store.set_state(&cell)?;
        }
        self.put(cell);
        Ok(())
    }

    /// Would giving `name` these deps create a cycle? Returns the path.
    pub(super) fn find_cycle(&self, name: &str, new_deps: &[String]) -> Option<Vec<String>> {
        // DFS from each new dep, following existing edges, looking for `name`.
        let mut stack: Vec<(String, Vec<String>)> = new_deps
            .iter()
            .map(|d| (d.clone(), vec![name.to_string(), d.clone()]))
            .collect();
        let mut visited = HashSet::new();
        while let Some((current, path)) = stack.pop() {
            if current == name {
                return Some(path);
            }
            if !visited.insert(current.clone()) {
                continue;
            }
            if let Some(cell) = self.cells.get(&current) {
                for dep in &cell.deps {
                    let mut p = path.clone();
                    p.push(dep.clone());
                    stack.push((dep.clone(), p));
                }
            }
        }
        None
    }
}

/// `kind[:opt|opt][~]` → a AnswerType; the template form of one.
pub(super) fn parse_answer_type_spec(spec: &str) -> Result<AnswerType, String> {
    // `~` lets a misfit land (noted); `!` refuses "Not found".
    let (mut spec, mut strict, mut allows_not_found) = (spec, true, true);
    loop {
        if let Some(s) = spec.strip_suffix('~') {
            (spec, strict) = (s, false);
        } else if let Some(s) = spec.strip_suffix('!') {
            (spec, allows_not_found) = (s, false);
        } else {
            break;
        }
    }
    let (kind, options) = match spec.split_once(':') {
        // A regex may contain `|`: the whole rest is the pattern.
        Some(("pattern", re)) => ("pattern", vec![re.to_string()]),
        Some((k, opts)) => (
            k,
            opts.split('|')
                .map(str::trim)
                .filter(|o| !o.is_empty())
                .map(str::to_string)
                .collect(),
        ),
        None => (spec, Vec::new()),
    };
    let mut answer_type = AnswerType::new(kind.parse()?, &options);
    answer_type.strict = strict;
    answer_type.allows_not_found = allows_not_found;
    answer_type.validate()?;
    Ok(answer_type)
}

pub(super) fn answer_type_spec(answer_type: &AnswerType) -> String {
    let mut spec = answer_type.kind.name().to_string();
    if !answer_type.options.is_empty() {
        spec.push(':');
        spec.push_str(&answer_type.options.join("|"));
    }
    if !answer_type.allows_not_found {
        spec.push('!');
    }
    if !answer_type.strict {
        spec.push('~');
    }
    spec
}
