//! Sheets: a board's other grids. A cell on one is an ordinary cell named
//! `Notes!A2`; this adds, renames, moves and deletes the sheets themselves.

use super::*;
use crate::model::{a1_coord, on_sheet, split_sheet};
use crate::store::Sheets;

impl Engine {
    /// The board's sheets, the first (unprefixed) one first.
    pub fn sheets(&self) -> Sheets {
        self.store.sheets()
    }

    /// A cell however a person or agent wrote it (`'My Notes'!$A$2`,
    /// `notes!A2`, `Sheet1!A2`, `status`) as recalc names it (`My Notes!A2`,
    /// `Notes!A2`, `A2`, `status`). Text that isn't a reference comes back as is.
    pub fn canonical(&self, text: &str) -> String {
        match crate::model::ref_name(text) {
            Some(name) => self.qualify(&name, None),
            None => text.trim().to_string(),
        }
    }

    /// `canonical` for a range too: `notes!A2:C5` → `Notes!A2:C5`.
    pub fn canonical_range(&self, text: &str) -> String {
        match text.split_once(':') {
            Some((from, to)) if crate::model::A1Ref::parse(to.trim()).is_some() => {
                format!("{}:{}", self.canonical(from), to.trim())
            }
            _ => self.canonical(text),
        }
    }

    /// The cell a reference names, seen from cell `host`: a reference as a
    /// source wrote it (`A2`, `Notes!A2`, `Sheet1!A2`, a range `A2:C5`, or
    /// with a `.code`-style suffix). An unqualified A1 reference means the
    /// host's own sheet, as in Excel; named cells are workbook-wide.
    pub fn qualify(&self, written: &str, host: Option<&str>) -> String {
        let host_sheet = host.and_then(|h| split_sheet(h).0);
        if let Some((from, to)) = written.split_once(':') {
            // A range: its first corner carries the sheet for both.
            return format!("{}:{to}", self.qualify(from, host));
        }
        let (base, meta) = crate::expr::split_meta(written);
        let name = match split_sheet(base) {
            (Some(sheet), local) => on_sheet(self.sheets.target(sheet), local),
            (None, local) if a1_coord(local).is_some() => on_sheet(host_sheet, local),
            (None, local) => local.to_string(),
        };
        match meta {
            Some(m) => format!("{name}.{m}"),
            None => name,
        }
    }

    /// Re-read the board's sheets when `name` is on one this engine hasn't
    /// seen: made by a write here, or by another process.
    pub(super) fn note_sheet(&mut self, name: &str) {
        if let (Some(sheet), _) = split_sheet(name) {
            if !self.sheets.others.iter().any(|s| s == sheet) {
                let _ = self.adopt_sheets();
            }
        }
    }

    /// The sheet a tool or command names, as an optional prefix: None for the
    /// first sheet (by its name or none given), the board's spelling otherwise.
    pub fn sheet_arg(&self, name: Option<&str>) -> Result<Option<String>> {
        match name.map(str::trim).filter(|n| !n.is_empty()) {
            None => Ok(None),
            Some(n) if self.sheets.find(n).is_none() => bail!("no sheet named {n}"),
            Some(n) => Ok(self.sheets.target(n).map(str::to_string)),
        }
    }

    pub fn add_sheet(&mut self, name: &str) -> Result<()> {
        self.store.add_sheet(name)?;
        self.adopt_sheets()?;
        Ok(())
    }

    pub fn move_sheet(&mut self, name: &str, index: usize) -> Result<()> {
        self.store.move_sheet(name, index)?;
        self.adopt_sheets()?;
        Ok(())
    }

    /// Rename a sheet; its cells come along under their new names, and
    /// every reference to it follows.
    pub fn rename_sheet(&mut self, from: &str, to: &str) -> Result<()> {
        let moved = self.cells_on(from);
        self.refuse_while_running(&moved, "renaming")?;
        let readers = self.readers_of(&moved);
        self.store.rename_sheet(from, to)?;
        self.adopt_sheets()?;
        // Out of the mirror first, so no recompute writes under the old name.
        for name in &moved {
            self.remove(name);
        }
        let mut arrived = Vec::new();
        if self.sheets.first != to {
            for name in &moved {
                let new = on_sheet(Some(to), split_sheet(name).1);
                if let Some(mut cell) = self.store.get(&new)? {
                    // Its unqualified references now mean the renamed sheet.
                    if let Some(src) = &cell.source {
                        cell.deps =
                            parse_deps(cell.kind, def_body(src), &|d| self.qualify(d, Some(&new)))
                                .0;
                    }
                    self.put(cell);
                    arrived.push(new);
                }
            }
        }
        self.retarget(from, Some(to))?;
        let readers: Vec<String> = readers
            .into_iter()
            .filter(|r| self.cells.contains_key(r))
            .collect();
        self.recompute(readers)
    }

    /// Copy a sheet to a new one after it: each cell as written (a formula,
    /// command or prompt; a value as it is), with its format and answer type.
    /// Agents' answers, marks and citations stay behind, so the copy starts
    /// fresh, as a workpaper for the next client does. Unqualified references
    /// in the copy mean its own sheet.
    pub fn duplicate_sheet(&mut self, from: &str, to: &str, by: Actor) -> Result<()> {
        let Some(found) = self.sheets.find(from).map(str::to_string) else {
            bail!("no sheet named {from}");
        };
        let prefix = self.sheets.target(&found).map(str::to_string);
        let mut sourced = Vec::new();
        let mut literals = Vec::new();
        let mut extras = Vec::new();
        for (name, cell) in &self.cells {
            let (sheet, local) = split_sheet(name);
            // Named cells belong to the board, not to a sheet.
            if sheet != prefix.as_deref() || crate::model::a1_coord(local).is_none() {
                continue;
            }
            let new = on_sheet(Some(to), local);
            match &cell.source {
                Some(src) => sourced.push((new.clone(), src.clone())),
                None => literals.push((new.clone(), cell.value.clone())),
            }
            extras.push((new, cell.format.clone(), cell.answer_type.clone()));
        }
        self.add_sheet(to)?;
        // Just after the one it copies, as Excel puts a copy.
        let names = self.sheets();
        if let Some(at) = names
            .others
            .iter()
            .position(|s| s.eq_ignore_ascii_case(&found))
        {
            self.move_sheet(to, at + 1)?;
        } else {
            self.move_sheet(to, 0)?;
        }
        self.write_literals(literals, by.clone())?;
        self.set_many(sourced, by)?;
        for (name, format, answer_type) in extras {
            if format.is_some() {
                self.set_format(&name, format)?;
            }
            if answer_type.is_some() {
                self.set_answer_type(&name, answer_type)?;
            }
        }
        Ok(())
    }

    /// Delete a sheet and its cells; references to it become `#REF!`.
    pub fn delete_sheet(&mut self, name: &str) -> Result<()> {
        let gone = self.cells_on(name);
        self.refuse_while_running(&gone, "deleting")?;
        let readers = self.readers_of(&gone);
        self.store.delete_sheet(name)?;
        self.adopt_sheets()?;
        for cell in &gone {
            self.remove(cell);
        }
        self.retarget(name, None)?;
        let readers: Vec<String> = readers
            .into_iter()
            .filter(|r| self.cells.contains_key(r))
            .collect();
        self.recompute(readers)
    }

    /// The cells outside `names` that read any of them.
    fn readers_of(&self, names: &[String]) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for name in names {
            for reader in self.dependents_of(name) {
                if !names.contains(&reader) && !out.contains(&reader) {
                    out.push(reader);
                }
            }
        }
        out
    }

    /// A moved cell's run would land at the old address, and its live
    /// session would be left there; as with rows, wait.
    fn refuse_while_running(&self, names: &[String], doing: &str) -> Result<()> {
        if let Some(busy) = names.iter().find(|n| {
            self.cells
                .get(*n)
                .is_some_and(|c| matches!(c.state, CellState::Computing | CellState::Queued))
        }) {
            bail!("{busy} is still running; stop it before {doing} its sheet");
        }
        if let Some(live) = self.live_session().filter(|l| names.contains(l)) {
            bail!("{live} has a live session; end it before {doing} its sheet");
        }
        Ok(())
    }

    /// Re-read the board's sheets. When their names changed (one added,
    /// renamed or deleted, here or by another process), references are
    /// resolved again: one written before its sheet existed, in other
    /// capitals, or to the first sheet by a name it has only now.
    pub(super) fn adopt_sheets(&mut self) -> Result<()> {
        let sheets = self.store.sheets();
        let names = |s: &crate::store::Sheets| {
            let mut n: Vec<String> = s.others.clone();
            n.sort();
            (s.first.clone(), n)
        };
        let changed = names(&sheets) != names(&self.sheets);
        self.sheets = sheets;
        if !changed {
            return Ok(());
        }
        let mut moved = Vec::new();
        let sources: Vec<(String, CellKind, String)> = self
            .cells
            .values()
            .filter_map(|c| {
                c.source
                    .as_ref()
                    .filter(|s| s.contains('!'))
                    .map(|s| (c.name.clone(), c.kind, s.clone()))
            })
            .collect();
        for (name, kind, src) in sources {
            let deps = parse_deps(kind, def_body(&src), &|d| self.qualify(d, Some(&name))).0;
            let mut cell = self.cells[&name].clone();
            if cell.deps != deps {
                cell.deps = deps;
                self.store.set_state(&cell)?;
                self.put(cell);
                moved.push(name);
            }
        }
        moved.retain(|n| matches!(self.cells.get(n).map(|c| c.kind), Some(CellKind::Expr)));
        if moved.is_empty() {
            return Ok(());
        }
        self.recompute(moved)
    }

    /// Another process added, renamed or deleted a sheet: take its sheets
    /// and bring every cell on them in line with the disk.
    pub(super) fn sheets_changed_elsewhere(&mut self) -> Result<()> {
        self.adopt_sheets()?;
        let mut names: std::collections::BTreeSet<String> = self
            .cells
            .keys()
            .filter(|n| n.contains('!'))
            .cloned()
            .collect();
        names.extend(
            self.store
                .list()?
                .into_iter()
                .map(|c| c.name)
                .filter(|n| n.contains('!')),
        );
        for name in names {
            self.reconcile(&name)?;
        }
        Ok(())
    }

    /// The names of the cells on sheet `sheet` (not the first).
    fn cells_on(&self, sheet: &str) -> Vec<String> {
        self.cells
            .keys()
            .filter(|n| split_sheet(n).0 == Some(sheet))
            .cloned()
            .collect()
    }

    /// Point every source that names sheet `from` at `to`, or at `#REF!` when
    /// it was deleted, and recompute what changed.
    fn retarget(&mut self, from: &str, to: Option<&str>) -> Result<()> {
        let mut changed = Vec::new();
        let sources: Vec<(String, String)> = self
            .cells
            .values()
            .filter_map(|c| c.source.clone().map(|s| (c.name.clone(), s)))
            .collect();
        for (name, src) in sources {
            let formula = self.cells[&name].kind == CellKind::Expr;
            let rewritten = super::structure::retarget_sheet(&src, from, to, formula);
            if rewritten == src {
                continue;
            }
            let mut cell = self.cells[&name].clone();
            cell.deps = parse_deps(cell.kind, def_body(&rewritten), &|d| {
                self.qualify(d, Some(&name))
            })
            .0;
            cell.source = Some(rewritten);
            self.store.put(&name, &cell)?;
            self.put(cell);
            changed.push(name);
        }
        changed.retain(|n| {
            matches!(
                self.cells.get(n).map(|c| c.kind),
                Some(CellKind::Expr | CellKind::Literal)
            )
        });
        if changed.is_empty() {
            return Ok(());
        }
        self.recompute(changed)
    }
}
