//! Board templates and CSV import.

use super::*;

impl Engine {
    /// Apply a board template: `name = content` lines, `#` comments, and
    /// `@series`/`@answer_type` lines, as `recalc save` writes. Content with a
    /// line break, or ending in a space, comes as a heredoc: `name <<END`, its
    /// lines as they are, then `END` alone.
    pub fn apply_template(&mut self, text: &str, by: Actor) -> Result<()> {
        // `@first Name`: what the saved board called its first sheet. An empty
        // board takes the name; otherwise references by it mean this board's first.
        let mut first_as: Option<String> = None;
        let retarget = |content: &str, alias: &Option<String>, first: &str| match alias {
            Some(a) if !a.eq_ignore_ascii_case(first) => super::structure::retarget_sheet(
                content,
                a,
                Some(first),
                content.trim_start().starts_with('='),
            ),
            _ => content.to_string(),
        };
        let mut lines = text.lines().enumerate();
        while let Some((n, line)) = lines.next() {
            if let Some((name, end)) = heredoc_start(line) {
                let mut body: Vec<&str> = Vec::new();
                loop {
                    match lines.next() {
                        Some((_, l)) if l == end => break,
                        Some((_, l)) => body.push(l),
                        None => bail!(
                            "line {}: {name}'s value never reaches its closing {end}",
                            n + 1
                        ),
                    }
                }
                let name = self.canonical(name);
                let content = retarget(&body.join("\n"), &first_as, &self.sheets.first);
                self.set(&name, &content, by.clone())?;
                continue;
            }
            let line = line.trim_end();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(first) = line.strip_prefix("@first ") {
                let first = first.trim().to_string();
                if self.cells.is_empty() && !first.eq_ignore_ascii_case(&self.sheets.first) {
                    self.rename_sheet(&self.sheets.first.clone(), &first)
                        .map_err(|e| anyhow::anyhow!("line {}: {e}", n + 1))?;
                }
                first_as = Some(first);
                continue;
            }
            // `@sheet Name`: a sheet, kept even when empty, in tab order.
            if let Some(sheet) = line.strip_prefix("@sheet ") {
                let sheet = sheet.trim();
                if self.sheets.find(sheet).is_none() {
                    self.add_sheet(sheet)
                        .map_err(|e| anyhow::anyhow!("line {}: {e}", n + 1))?;
                }
                continue;
            }
            if let Some(names) = line.strip_prefix("@series ") {
                for name in name_list(names) {
                    self.set_series(&self.canonical(&name), true)?;
                }
                continue;
            }
            // `@answer_type names… = kind[:opt|opt][~]`; `~` makes it lenient. Names
            // come first so an option may contain spaces.
            if let Some(rest) = line.strip_prefix("@answer_type ") {
                let (names, spec) = rest.split_once(" = ").ok_or_else(|| {
                    anyhow::anyhow!(
                        "line {}: expected `@answer_type names… = kind[:opt|opt]`",
                        n + 1
                    )
                })?;
                let answer_type = parse_answer_type_spec(spec.trim())
                    .map_err(|e| anyhow::anyhow!("line {}: {e}", n + 1))?;
                for name in name_list(names) {
                    self.set_answer_type(&self.canonical(&name), Some(answer_type.clone()))?;
                }
                continue;
            }
            let (name, content) = line
                .split_once(" = ")
                .or_else(|| line.strip_suffix(" =").map(|n| (n, "")))
                .ok_or_else(|| anyhow::anyhow!("line {}: expected `name = content`", n + 1))?;
            let name = self.canonical(name);
            let content = retarget(content, &first_as, &self.sheets.first);
            self.set(&name, &content, by.clone())?;
        }
        Ok(())
    }

    /// The board as template text; the inverse of `apply_template`.
    pub fn save_template(&self, note: Option<&str>) -> String {
        let mut out = String::new();
        if let Some(note) = note {
            out.push_str(&format!("# {note}\n"));
        }
        if self.sheets.first != crate::store::Sheets::default().first {
            out.push_str(&format!("@first {}\n", self.sheets.first));
        }
        for sheet in &self.sheets.others {
            out.push_str(&format!("@sheet {sheet}\n"));
        }
        let mut names: Vec<&String> = self.cells.keys().collect();
        names.sort();
        let mut series = Vec::new();
        // One line per answer-type spec.
        let mut answer_types: Vec<(String, Vec<String>)> = Vec::new();
        for name in names {
            let cell = &self.cells[name];
            // In lists, a name with a space is quoted: `'My Notes'!A2`.
            if cell.series {
                series.push(crate::model::ref_text(&cell.name));
            }
            if let Some(answer_type) = &cell.answer_type {
                let spec = answer_type_spec(answer_type);
                let name = crate::model::ref_text(&cell.name);
                match answer_types.iter_mut().find(|(s, _)| *s == spec) {
                    Some((_, cells)) => cells.push(name),
                    None => answer_types.push((spec, vec![name])),
                }
            }
            let content = cell.source.as_deref().unwrap_or(&cell.value);
            if content.contains('\n') || content.ends_with(char::is_whitespace) {
                // A closing word none of its lines is.
                let end = (1..)
                    .map(|i| {
                        if i == 1 {
                            "END".to_string()
                        } else {
                            format!("END{i}")
                        }
                    })
                    .find(|end| !content.lines().any(|l| l == end))
                    .expect("some END is free");
                out.push_str(&format!("{} <<{end}\n{content}\n{end}\n", cell.name));
            } else if content.is_empty() {
                out.push_str(&format!("{} =\n", cell.name));
            } else {
                out.push_str(&format!("{} = {content}\n", cell.name));
            }
        }
        if !series.is_empty() {
            out.push_str(&format!("@series {}\n", series.join(" ")));
        }
        for (spec, cells) in answer_types {
            out.push_str(&format!("@answer_type {} = {spec}\n", cells.join(" ")));
        }
        out
    }

    /// Import CSV with its top-left field at `at`, every field as a literal.
    /// Returns undo pairs.
    pub fn import_csv(&mut self, text: &str, at: &str, by: Actor) -> Result<Vec<(String, String)>> {
        let Some((sheet, c0, r0)) = cell_coord(at) else {
            bail!("import needs an A1 anchor cell, got {at:?}");
        };
        let mut entries = Vec::new();
        for (dr, row) in parse_csv(text).iter().enumerate() {
            for (dc, field) in row.iter().enumerate() {
                entries.push((
                    on_sheet(sheet, &a1_name(c0 + dc as u32, r0 + dr as u32)),
                    field.clone(),
                ));
            }
        }
        self.write_literals(entries, by)
    }
}

/// Minimal RFC 4180: quoted fields may hold commas, newlines, and
/// ""-escaped quotes; \r\n and \n both end records. A final newline is a
/// terminator, not an empty record.
pub(super) fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = vec![vec![String::new()]];
    let mut chars = text.chars().peekable();
    let mut quoted = false;
    while let Some(c) = chars.next() {
        let row = rows.last_mut().expect("rows starts non-empty");
        let field = row.last_mut().expect("rows start with one field");
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => quoted = false,
                _ => field.push(c),
            }
        } else {
            match c {
                '"' if field.is_empty() => quoted = true,
                ',' => row.push(String::new()),
                '\r' if chars.peek() == Some(&'\n') => {}
                '\n' => rows.push(vec![String::new()]),
                _ => field.push(c),
            }
        }
    }
    if rows.last().is_some_and(|r| r.len() == 1 && r[0].is_empty()) {
        rows.pop();
    }
    rows
}

/// `name <<END` opens a heredoc: the cell's name and the word that closes it.
fn heredoc_start(line: &str) -> Option<(&str, &str)> {
    let (name, end) = line.trim_end().split_once(" <<")?;
    (crate::model::ref_name(name).is_some()
        && !end.is_empty()
        && end.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
    .then_some((name, end))
}

/// The names in a `@series`/`@answer_type` list: space-separated, a quoted
/// sheet keeping its spaces (`'My Notes'!A2 B3`).
fn name_list(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in text.chars() {
        match c {
            '\'' => {
                quoted = !quoted;
                current.push(c);
            }
            c if c.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    names.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        names.push(current);
    }
    names
}
