//! Structural edits: sort, fill, insert and delete rows or columns, with references rewritten.

use super::*;

impl Engine {
    /// Sort rows `first..=last` by a column, stably: numbers before text,
    /// blanks last. Cells move whole; references into a moved row follow it and
    /// ranges stay put. Returns the moves as (from, to), for undo.
    pub fn sort_rows(
        &mut self,
        sheet: Option<&str>,
        col: u32,
        first: u32,
        last: u32,
        descending: bool,
        by: Actor,
    ) -> Result<Vec<(u32, u32)>> {
        if first == 0 || last < first {
            bail!("sort needs a 1-based row range");
        }
        let mut rows: Vec<u32> = (first..=last).collect();
        let key = |row: u32| -> SortKey {
            let value = self
                .cells
                .get(&on_sheet(sheet, &a1_name(col, row)))
                .map(|c| c.value.trim().to_string())
                .unwrap_or_default();
            SortKey::of(&value)
        };
        let keys: HashMap<u32, SortKey> = rows.iter().map(|&r| (r, key(r))).collect();
        rows.sort_by(|a, b| {
            let (ka, kb) = (&keys[a], &keys[b]);
            // Blanks stay last whichever way the rest runs.
            match (ka, kb) {
                (SortKey::Blank, SortKey::Blank) => std::cmp::Ordering::Equal,
                (SortKey::Blank, _) => std::cmp::Ordering::Greater,
                (_, SortKey::Blank) => std::cmp::Ordering::Less,
                _ if descending => kb.cmp(ka),
                _ => ka.cmp(kb),
            }
        });
        // rows[i] is the row that lands at first + i.
        let moves: Vec<(u32, u32)> = rows
            .iter()
            .enumerate()
            .map(|(i, &row)| (row, first + i as u32))
            .filter(|(from, to)| from != to)
            .collect();
        self.move_rows(sheet, &moves, by)?;
        Ok(moves)
    }

    /// Permute rows by (from, to) pairs, as a sort does. A sort's pairs,
    /// swapped, undo it.
    pub fn move_rows(
        &mut self,
        sheet: Option<&str>,
        moves: &[(u32, u32)],
        by: Actor,
    ) -> Result<()> {
        let dest: HashMap<u32, u32> = moves.iter().copied().collect();
        let froms: HashSet<u32> = dest.keys().copied().collect();
        let tos: HashSet<u32> = dest.values().copied().collect();
        if froms.len() != moves.len() || froms != tos || froms.contains(&0) {
            bail!("moving rows needs a permutation of 1-based rows");
        }
        if dest.is_empty() {
            return Ok(());
        }
        self.restructure(
            sheet,
            Axis::Rows,
            by,
            |r| Some(*dest.get(&r).unwrap_or(&r)),
            |r, _| r,
        )
        .map(|_| ())
    }

    /// Fill a rectangle down or right (⌘D / ⌘R) from each lane's first cell,
    /// shifting A1 refs. A single-row (or -column) selection fills from the cell
    /// above (or left). Returns undo pairs.
    pub fn fill(
        &mut self,
        from: &str,
        to: &str,
        dir: Fill,
        by: Actor,
    ) -> Result<Vec<(String, String)>> {
        let (Some((sheet, c1, r1)), Some((to_sheet, c2, r2))) = (cell_coord(from), cell_coord(to))
        else {
            bail!("fill needs an A1 range");
        };
        if sheet != to_sheet {
            bail!("fill needs both corners on one sheet");
        }
        // Lanes run across the fill; `along` is the fill direction.
        let (lanes, along) = match dir {
            Fill::Down => ((c1.min(c2), c1.max(c2)), (r1.min(r2), r1.max(r2))),
            Fill::Right => ((r1.min(r2), r1.max(r2)), (c1.min(c2), c1.max(c2))),
        };
        let name_at = |lane: u32, i: u32| match dir {
            Fill::Down => on_sheet(sheet, &a1_name(lane, i)),
            Fill::Right => on_sheet(sheet, &a1_name(i, lane)),
        };
        let (src_i, first) = if along.0 == along.1 {
            if along.0 == 1 {
                bail!("nothing above or left of the selection to fill from");
            }
            (along.0 - 1, along.0)
        } else {
            (along.0, along.0 + 1)
        };

        let mut changed = Vec::new();
        for lane in lanes.0..=lanes.1 {
            let source = name_at(lane, src_i);
            let Some(content) = self.content(&source) else {
                continue; // nothing to fill from in this lane
            };
            // Literals copy verbatim; their text holds no references.
            let is_literal = self.cells[&source].source.is_none();
            for i in first..=along.1 {
                let target = name_at(lane, i);
                let shifted = if is_literal {
                    content.clone()
                } else {
                    let (dc, dr) = match dir {
                        Fill::Down => (0, i64::from(i - src_i)),
                        Fill::Right => (i64::from(i - src_i), 0),
                    };
                    shift_refs(&content, dc, dr)
                };
                let prev = self.content(&target).unwrap_or_default();
                // A re-fill keeps verified cells.
                if prev == shifted || self.cells.get(&target).is_some_and(Cell::verified) {
                    continue;
                }
                // A per-cell failure (a cycle) leaves that cell erroring and fills the
                // rest; cells already set must still reach undo.
                if self.set(&target, &shifted, by.clone()).is_ok() {
                    changed.push((target, prev));
                }
            }
        }
        Ok(changed)
    }

    /// Insert `n` empty rows (or columns) before `at`. Cells move whole and
    /// every reference follows, ranges included. Nothing recomputes.
    pub fn insert(
        &mut self,
        sheet: Option<&str>,
        axis: Axis,
        at: u32,
        n: u32,
        by: Actor,
    ) -> Result<()> {
        if at == 0 || n == 0 {
            bail!("insert needs a 1-based index and a positive count");
        }
        self.restructure(
            sheet,
            axis,
            by,
            |i| if i >= at { Some(i + n) } else { Some(i) },
            |i, _| {
                if i >= at {
                    i + n
                } else {
                    i
                }
            },
        )
        .map(|_| ())
    }

    /// Delete rows (or columns) `at..at+n`; the rest close the gap. A range
    /// with a corner in the band shrinks; one wholly inside becomes `{#REF!}`.
    /// Returns what it took, for `undelete`.
    pub fn delete(
        &mut self,
        sheet: Option<&str>,
        axis: Axis,
        at: u32,
        n: u32,
        by: Actor,
    ) -> Result<Removed> {
        if at == 0 || n == 0 {
            bail!("delete needs a 1-based index and a positive count");
        }
        let end = at + n; // exclusive
        self.restructure(
            sheet,
            axis,
            by,
            move |i| {
                if i < at {
                    Some(i)
                } else if i < end {
                    None
                } else {
                    Some(i - n)
                }
            },
            // A corner in the band lands on its edge: near on the row that moves up,
            // far on the row before, so the range gains no row. A range wholly inside
            // inverts, giving #REF!
            move |i, far| {
                if i < at {
                    i
                } else if i < end {
                    if far {
                        at - 1
                    } else {
                        at
                    }
                } else {
                    i - n
                }
            },
        )
    }

    /// Undo a `delete`: re-insert the band so references follow, then restore
    /// the deleted cells and the sources the delete rewrote, which shifted text
    /// can't recover. A source edited since the delete is kept.
    pub fn undelete(
        &mut self,
        sheet: Option<&str>,
        axis: Axis,
        at: u32,
        n: u32,
        removed: &Removed,
        by: Actor,
    ) -> Result<()> {
        let untouched: Vec<&Rewritten> = removed
            .rewritten
            .iter()
            .filter(|w| {
                self.cells.get(&w.after).and_then(|c| c.source.as_deref())
                    == Some(w.after_source.as_str())
            })
            .collect();
        let befores: Vec<String> = untouched.iter().map(|w| w.before.clone()).collect();
        // What each agent or action cell coming back was, before the recompute
        // below marks it out of date: its inputs come back as they were.
        let mut were: Vec<(String, CellState)> = untouched
            .iter()
            .filter_map(|w| Some((w.before.clone(), self.cells.get(&w.after)?.state.clone())))
            .collect();
        were.extend(
            removed
                .cells
                .iter()
                .map(|c| (c.name.clone(), c.state.clone())),
        );
        self.insert(sheet, axis, at, n, by)?;
        for w in untouched {
            let Some(mut cell) = self.cells.get(&w.before).cloned() else {
                continue;
            };
            cell.source = Some(w.before_source.clone());
            cell.deps = parse_deps(cell.kind, def_body(&w.before_source), &|d| {
                self.qualify(d, Some(&cell.name))
            })
            .0;
            self.store.put(&cell.name, &cell)?;
            self.put(cell);
        }
        let mut roots = befores;
        for cell in &removed.cells {
            self.store.put(&cell.name, cell)?;
            self.put(cell.clone());
            self.bump_gen(&cell.name);
            roots.push(cell.name.clone());
        }
        // Restored sources and their readers recompute; agent cells keep their
        // answers.
        roots.retain(|n| {
            matches!(
                self.cells.get(n).map(|c| c.kind),
                Some(CellKind::Literal | CellKind::Expr | CellKind::Command)
            )
        });
        if !roots.is_empty() {
            self.recompute(roots)?;
        }
        for (name, was) in were {
            let Some(cell) = self.cells.get_mut(&name) else {
                continue;
            };
            let held = matches!(cell.kind, CellKind::AgentCall | CellKind::Action);
            if held && cell.state == CellState::Stale && was != CellState::Stale {
                cell.state = was;
                self.store.set_state(cell)?;
                self.touched.insert(name);
            }
        }
        Ok(())
    }

    /// `cell_map` sends an index to its new one (None = removed); `corner_map`
    /// does the same for a range corner, given whether it is the far edge (see
    /// `remap_refs`). Refused while anything runs or has a live session, since
    /// a moved cell's result, or its session, would be left at the old address.
    pub(super) fn restructure(
        &mut self,
        sheet: Option<&str>,
        axis: Axis,
        by: Actor,
        cell_map: impl Fn(u32) -> Option<u32>,
        corner_map: impl Fn(u32, bool) -> u32,
    ) -> Result<Removed> {
        self.catch_up()?;
        if let Some(busy) = self
            .cells
            .values()
            .find(|c| matches!(c.state, CellState::Computing | CellState::Queued))
        {
            bail!(
                "{} is still running; stop it before moving rows or columns",
                busy.name
            );
        }
        if let Some(live) = self.live_session() {
            bail!("{live} has a live session; end it before moving rows or columns");
        }
        let map_coord = |c: u32, r: u32| -> Option<(u32, u32)> {
            match axis {
                Axis::Rows => cell_map(r).map(|r| (c, r)),
                Axis::Cols => cell_map(c).map(|c| (c, r)),
            }
        };
        let map_corner = |c: u32, r: u32, c_far: bool, r_far: bool| -> (u32, u32) {
            match axis {
                Axis::Rows => (c, corner_map(r, r_far)),
                Axis::Cols => (corner_map(c, c_far), r),
            }
        };
        // Only this sheet's cells move.
        let map_name = |name: &str| -> Option<String> {
            match cell_coord(name) {
                Some((s, c, r)) if s == sheet => {
                    map_coord(c, r).map(|(c, r)| on_sheet(sheet, &a1_name(c, r)))
                }
                _ => Some(name.to_string()),
            }
        };

        // Plan every new address and source before touching disk.
        let mut plan: Vec<(String, Cell)> = Vec::new();
        let mut gone: Vec<String> = Vec::new();
        let mut removed = Removed::default();
        for cell in self.cells.values() {
            let Some(new_name) = map_name(&cell.name) else {
                gone.push(cell.name.clone());
                removed.cells.push(cell.clone());
                continue;
            };
            let mut moved = cell.clone();
            if let Some(src) = &cell.source {
                // A reference moves when it points into this sheet, as seen from its cell.
                let host = split_sheet(&cell.name).0;
                let in_scope = |written: Option<&str>| -> bool {
                    written.map_or(host, |w| self.sheets.target(w)) == sheet
                };
                let rewritten = remap_refs(src, &|c, r| map_coord(c, r), &map_corner, &in_scope);
                if rewritten != *src {
                    moved.source = Some(rewritten.clone());
                    moved.deps = parse_deps(moved.kind, def_body(&rewritten), &|d| {
                        self.qualify(d, Some(&moved.name))
                    })
                    .0;
                    removed.rewritten.push(Rewritten {
                        before: cell.name.clone(),
                        before_source: src.clone(),
                        after: new_name.clone(),
                        after_source: rewritten,
                    });
                }
            }
            moved.name = new_name;
            plan.push((cell.name.clone(), moved));
        }
        self.log_as(
            &by,
            format!(
                "moved {} {}",
                match axis {
                    Axis::Rows => "rows",
                    Axis::Cols => "columns",
                },
                if gone.is_empty() {
                    "(insert)"
                } else {
                    "(delete)"
                }
            ),
        );
        // Write each address over what it held; delete only addresses left empty.
        for name in &gone {
            self.store.delete_history(name)?;
        }
        let moves: Vec<(String, String)> = plan
            .iter()
            .filter(|(old, c)| *old != c.name)
            .map(|(old, c)| (old.clone(), c.name.clone()))
            .collect();
        self.store.move_histories(&moves)?;
        let landing: HashSet<&str> = plan.iter().map(|(_, c)| c.name.as_str()).collect();
        let emptied: Vec<String> = gone
            .iter()
            .chain(moves.iter().map(|(old, _)| old))
            .filter(|n| !landing.contains(n.as_str()))
            .cloned()
            .collect();
        self.store.delete_many(&emptied)?;
        let puts: Vec<(&Cell, Option<&Cell>)> = plan
            .iter()
            .filter(|(old, c)| *old != c.name || c.source != self.cells[old].source)
            .map(|(_, c)| (c, self.cells.get(&c.name)))
            .collect();
        self.store.put_many(&puts)?;
        // Rebuild the mirror from the plan; every address is fresh.
        let mut old_names: Vec<String> = self.cells.keys().cloned().collect();
        old_names.append(&mut gone);
        for name in old_names {
            self.remove(&name);
        }
        for (_, cell) in plan {
            self.put(cell);
        }
        self.run_gen.clear();
        self.watch_dirty = true;
        // Formulas and commands whose references moved recompute, so a lost row
        // or `{#REF!}` shows; agent cells don't re-run.
        let roots: Vec<String> = removed
            .rewritten
            .iter()
            .map(|w| w.after.clone())
            .filter(|n| {
                matches!(
                    self.cells.get(n).map(|c| c.kind),
                    Some(CellKind::Expr | CellKind::Command)
                )
            })
            .collect();
        if !roots.is_empty() {
            self.recompute(roots)?;
        }
        Ok(removed)
    }
}

/// Split `{A2:A9.mark}` into the range and `.mark`, so fill and delete move
/// the corners and keep the suffix.
pub(super) fn split_range_meta(body: &str) -> (&str, &str) {
    match body.rsplit_once('.') {
        Some((range, m)) if range.contains(':') && expr::META.contains(&m) => {
            (range, &body[range.len()..])
        }
        _ => (body, ""),
    }
}

/// Whether a brace body is a formula's list of references (`{a, B2.mark}`)
/// rather than shell braces.
pub(super) fn is_ref_list(body: &str) -> bool {
    body.contains(',')
        && body.split(',').all(|it| {
            let (core, _) = expr::split_meta(it.trim());
            crate::model::ref_name(core).is_some()
        })
}

/// Whether the rewrite just wrote a deleted range (`{#REF!`), which takes no
/// suffix.
pub(super) fn rewritten_is_ref(out: &str) -> bool {
    out.ends_with("{#REF!")
}

/// Shift every A1 reference in a def source by (dcol, drow), as fill does.
/// A `$` anchor pins that axis; named cells are absolute and `${...}` is
/// shell syntax, so both pass through.
pub(super) fn shift_refs(src: &str, dcol: i64, drow: i64) -> String {
    // A ref the shift would push off the sheet stays as written. A sheet
    // prefix stays as written; its A1 part shifts, as in Excel.
    let shift_name = |name: &str| -> Option<String> {
        let (_, rest) = crate::model::ref_sheet(name)?;
        let prefix = &name.trim()[..name.trim().len() - rest.len()];
        Some(format!(
            "{prefix}{}",
            A1Ref::parse(rest)?.shifted(dcol, drow)?
        ))
    };
    let mut out = String::new();
    let mut rest = src;
    while let Some(open) = rest.find('{') {
        let shell_var = rest[..open].ends_with('$');
        out.push_str(&rest[..=open]);
        rest = &rest[open + 1..];
        if shell_var {
            continue;
        }
        let Some(close) = rest.find('}') else { break };
        let (body, meta) = split_range_meta(&rest[..close]);
        let one = |item: &str| -> Option<String> {
            match expr::split_meta(item.trim()) {
                (a, Some(m)) => shift_name(a.trim()).map(|a| format!("{a}.{m}")),
                (a, None) => shift_name(a),
            }
        };
        let shifted = if is_ref_list(body) {
            // `{a, B2}`: a formula's list; each item shifts on its own.
            let items: Vec<String> = body
                .split(',')
                .map(|it| one(it).unwrap_or_else(|| it.trim().to_string()))
                .collect();
            let same = items
                .iter()
                .zip(body.split(','))
                .all(|(n, o)| n == o.trim());
            (!same).then(|| items.join(", "))
        } else if let Some((a, b)) = body.split_once(':') {
            let (a, b) = (a.trim(), b.trim());
            if A1Ref::parse(b).is_some() {
                // {A1:B3} is a range: shift both corners.
                match (shift_name(a), shift_name(b)) {
                    (Some(a), Some(b)) => Some(format!("{a}:{b}")),
                    _ => None,
                }
            } else {
                // {A2:file} is a form suffix on one ref.
                shift_name(a).map(|a| format!("{a}:{b}"))
            }
        } else {
            one(body)
        };
        out.push_str(&shifted.unwrap_or_else(|| body.to_string()));
        out.push_str(meta);
        out.push('}');
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

/// What a cell sorts as: numbers before text, text case-insensitively.
#[derive(Debug, PartialEq)]
pub(super) enum SortKey {
    Num(f64),
    Text(String),
    Blank,
}

impl SortKey {
    fn of(value: &str) -> SortKey {
        if value.is_empty() {
            SortKey::Blank
        } else if let Some(n) = crate::expr::parse_number(value) {
            // As formulas read it: $1,200 and 5% sort as numbers.
            SortKey::Num(n)
        } else {
            SortKey::Text(value.to_lowercase())
        }
    }
}

impl Eq for SortKey {}

impl PartialOrd for SortKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SortKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering::*;
        match (self, other) {
            (SortKey::Num(a), SortKey::Num(b)) => a.partial_cmp(b).unwrap_or(Equal),
            (SortKey::Num(_), SortKey::Text(_)) => Less,
            (SortKey::Text(_), SortKey::Num(_)) => Greater,
            (SortKey::Text(a), SortKey::Text(b)) => a.cmp(b),
            (SortKey::Blank, SortKey::Blank) => Equal,
            (SortKey::Blank, _) => Greater,
            (_, SortKey::Blank) => Less,
        }
    }
}

/// Rewrite every A1 reference in a source through a coordinate map, keeping
/// `$` anchors and suffixes. Range corners go through `corner`, which never
/// removes. A single ref to a removed cell becomes `{#REF!}` rather than
/// reading whatever moved into its address.
pub(super) fn remap_refs(
    src: &str,
    cell: &dyn Fn(u32, u32) -> Option<(u32, u32)>,
    corner: &dyn Fn(u32, u32, bool, bool) -> (u32, u32),
    in_scope: &dyn Fn(Option<&str>) -> bool,
) -> String {
    // A reference as written: its sheet prefix (kept as written) and A1 part.
    let parts = |name: &str| -> Option<(String, Option<String>, A1Ref)> {
        let name = name.trim();
        let (sheet, rest) = crate::model::ref_sheet(name)?;
        let r = A1Ref::parse(rest)?;
        Some((name[..name.len() - rest.len()].to_string(), sheet, r))
    };
    // Move a reference's A1 part, its prefix and anchors as written.
    let relabel = |name: &str, f: &dyn Fn(u32, u32) -> Option<(u32, u32)>| -> Option<String> {
        let (prefix, _, r) = parts(name)?;
        let (c, row) = f(r.col, r.row)?;
        let letters = a1_name(c, 1);
        let letters = letters.trim_end_matches('1');
        Some(format!(
            "{prefix}{}{letters}{}{row}",
            if r.col_fixed { "$" } else { "" },
            if r.row_fixed { "$" } else { "" }
        ))
    };
    // `relabel` a reference into the restructured sheet; others stay.
    let rename = |name: &str, f: &dyn Fn(u32, u32) -> Option<(u32, u32)>| -> Option<String> {
        let (_, sheet, _) = parts(name)?;
        if !in_scope(sheet.as_deref()) {
            return Some(name.trim().to_string());
        }
        relabel(name, f)
    };
    let is_ref = |name: &str| parts(name).is_some();
    let mut out = String::new();
    let mut rest = src;
    while let Some(open) = rest.find('{') {
        let shell_var = rest[..open].ends_with('$');
        out.push_str(&rest[..=open]);
        rest = &rest[open + 1..];
        if shell_var {
            continue;
        }
        let Some(close) = rest.find('}') else { break };
        let (body, meta) = split_range_meta(&rest[..close]);
        let one = |item: &str| -> String {
            let item = item.trim();
            let (a, m) = expr::split_meta(item);
            let a = a.trim();
            match parts(a) {
                None => item.to_string(),
                Some(_) => match (rename(a, cell), m) {
                    (Some(a), Some(m)) => format!("{a}.{m}"),
                    (Some(a), None) => a,
                    (None, _) => "#REF!".to_string(),
                },
            }
        };
        let rewritten = if is_ref_list(body) {
            // `{a, B2}`: a formula's list; each item moves on its own.
            let items: Vec<String> = body.split(',').map(one).collect();
            let same = items
                .iter()
                .zip(body.split(','))
                .all(|(n, o)| n == o.trim());
            (!same).then(|| items.join(", "))
        } else if let Some((a, b)) = body.split_once(':') {
            let (a, b) = (a.trim(), b.trim());
            if let (Some((_, a_sheet, ra)), Some((_, _, rb))) = (parts(a), parts(b)) {
                // A range is in the first corner's sheet; the second shares it.
                if !in_scope(a_sheet.as_deref()) {
                    None
                } else {
                    // Which corner is the far edge on each axis: the larger
                    // index, or the second as written when they're level.
                    let (a_far, b_far) = (
                        (ra.col > rb.col, ra.row > rb.row),
                        (rb.col >= ra.col, rb.row >= ra.row),
                    );
                    let near_a = |c, r| Some(corner(c, r, a_far.0, a_far.1));
                    let near_b = |c, r| Some(corner(c, r, b_far.0, b_far.1));
                    match (relabel(a, &near_a), relabel(b, &near_b)) {
                        (Some(na), Some(nb)) => {
                            let (pa, pb) = (parts(&na).map(|p| p.2), parts(&nb).map(|p| p.2));
                            // A range whose far edge passed its near one, or
                            // left the sheet, was wholly deleted.
                            let empty = match (pa, pb) {
                                (Some(pa), Some(pb)) => {
                                    let (lo, hi) = if a_far.1 { (&pb, &pa) } else { (&pa, &pb) };
                                    let (lo_c, hi_c) =
                                        if a_far.0 { (&pb, &pa) } else { (&pa, &pb) };
                                    hi.row < lo.row || hi_c.col < lo_c.col
                                }
                                _ => true,
                            };
                            Some(if empty {
                                "#REF!".to_string()
                            } else {
                                format!("{na}:{nb}")
                            })
                        }
                        _ => None,
                    }
                }
            } else {
                rename(a, cell).map(|a| format!("{a}:{b}"))
            }
        } else {
            let (a, meta) = expr::split_meta(body.trim());
            is_ref(a).then(|| match (rename(a, cell), meta) {
                (Some(a), Some(m)) => format!("{a}.{m}"),
                (Some(a), None) => a,
                (None, _) => "#REF!".to_string(),
            })
        };
        out.push_str(&rewritten.unwrap_or_else(|| body.to_string()));
        if !rewritten_is_ref(&out) {
            out.push_str(meta);
        }
        out.push('}');
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

/// Rewrite every reference that names sheet `from` (whatever its capitals):
/// to `to` when renamed, `#REF!` when the sheet is deleted (`to` is None).
/// Braced references move in every kind of source; a formula's bare
/// `Notes!A2` moves too (`formula`), but never text inside its strings, and
/// a command's or prompt's plain words stay as written.
pub(super) fn retarget_sheet(src: &str, from: &str, to: Option<&str>, formula: bool) -> String {
    let names_it = |item: &str| -> bool {
        crate::model::ref_sheet(item.trim())
            .and_then(|(sheet, _)| sheet)
            .is_some_and(|s| s.eq_ignore_ascii_case(from))
    };
    // `item` with its sheet prefix swapped; None when it doesn't name `from`.
    let swap = |item: &str| -> Option<String> {
        let (sheet, rest) = crate::model::ref_sheet(item.trim())?;
        if !sheet.is_some_and(|s| s.eq_ignore_ascii_case(from)) {
            return None;
        }
        Some(match to {
            Some(to) => format!("{}!{rest}", crate::model::quote_sheet(to)),
            None => "#REF!".to_string(),
        })
    };
    let bare = |text: &str| -> String {
        if formula {
            retarget_bare(text, from, to)
        } else {
            text.to_string()
        }
    };
    let mut out = String::new();
    let mut rest = src;
    while let Some(open) = rest.find('{') {
        let shell_var = rest[..open].ends_with('$');
        out.push_str(&bare(&rest[..=open]));
        rest = &rest[open + 1..];
        let Some(close) = rest.find('}') else { break };
        let body = &rest[..close];
        let rewritten = if shell_var {
            body.to_string()
        } else if is_ref_list(body) {
            body.split(',')
                .map(|it| {
                    let (a, m) = expr::split_meta(it.trim());
                    match (swap(a), m, to) {
                        (Some(s), Some(m), Some(_)) => format!("{s}.{m}"),
                        (Some(s), _, _) => s,
                        (None, _, _) => it.trim().to_string(),
                    }
                })
                .collect::<Vec<_>>()
                .join(", ")
        } else if !names_it(body) {
            body.to_string()
        } else if to.is_none() {
            "#REF!".to_string()
        } else {
            // A single ref, a suffixed one (`:raw`, `.code`) or a range whose
            // second corner may name the sheet too (`Notes!A1:Notes!A2`).
            let swapped = swap(body).unwrap_or_else(|| body.to_string());
            match swapped.split_once(':') {
                Some((first, second)) => match swap(second) {
                    Some(second) => format!("{first}:{second}"),
                    None => swapped,
                },
                None => swapped,
            }
        };
        out.push_str(&rewritten);
        out.push('}');
        rest = &rest[close + 1..];
    }
    out.push_str(&bare(rest));
    out
}

/// A formula's text outside braces: bare `Notes!A2` and `'My Notes'!A2`
/// naming `from` are retargeted; strings and everything else are copied.
fn retarget_bare(text: &str, from: &str, to: Option<&str>) -> String {
    // After a sheet: `!` (not `!=`), then an A1 reference with any anchors.
    let a1_after = |at: usize| -> Option<usize> {
        let tail = text.get(at..)?;
        if !tail.starts_with('!') || tail.starts_with("!=") {
            return None;
        }
        let len = tail[1..]
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '$')
            .unwrap_or(tail.len() - 1);
        A1Ref::parse(&tail[1..1 + len]).map(|_| at + 1 + len)
    };
    let replace = |out: &mut String, sheet: &str, whole: &str, a1: &str| {
        if !sheet.eq_ignore_ascii_case(from) {
            out.push_str(whole);
            return;
        }
        match to {
            Some(to) => {
                out.push_str(&crate::model::quote_sheet(to));
                out.push('!');
                out.push_str(a1);
            }
            None => out.push_str("#REF!"),
        }
    };
    let mut out = String::new();
    let mut i = 0;
    while let Some(c) = text[i..].chars().next() {
        if c == '\'' {
            // A quoted sheet (`'It''s'!A1`) or a string.
            let mut j = i + 1;
            let mut sheet = String::new();
            let mut closed = None;
            while let Some(d) = text[j..].chars().next() {
                if d == '\'' {
                    if text[j + 1..].starts_with('\'') {
                        sheet.push('\'');
                        j += 2;
                        continue;
                    }
                    closed = Some(j);
                    break;
                }
                sheet.push(d);
                j += d.len_utf8();
            }
            if let Some(end) = closed.and_then(|j| a1_after(j + 1)) {
                let bang = closed.expect("closed") + 1;
                replace(&mut out, &sheet, &text[i..end], &text[bang + 1..end]);
                i = end;
                continue;
            }
            // A string: copied to its closing quote, as the lexer reads it.
            let close = text[i + 1..]
                .find('\'')
                .map_or(text.len(), |k| i + 1 + k + 1);
            out.push_str(&text[i..close]);
            i = close;
            continue;
        }
        if c == '"' {
            let close = text[i + 1..]
                .find('"')
                .map_or(text.len(), |k| i + 1 + k + 1);
            out.push_str(&text[i..close]);
            i = close;
            continue;
        }
        let word_start = c.is_ascii_alphabetic() || c == '_';
        let after_word = text[..i]
            .chars()
            .next_back()
            .is_some_and(|p| p.is_ascii_alphanumeric() || p == '_' || p == '.');
        if word_start && !after_word {
            let len = text[i..]
                .find(|d: char| !d.is_ascii_alphanumeric() && d != '_')
                .unwrap_or(text.len() - i);
            let word = &text[i..i + len];
            if let Some(end) = a1_after(i + len) {
                replace(&mut out, word, &text[i..end], &text[i + len + 1..end]);
                i = end;
                continue;
            }
            out.push_str(word);
            i += len;
            continue;
        }
        out.push(c);
        i += c.len_utf8();
    }
    out
}
