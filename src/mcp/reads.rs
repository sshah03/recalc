//! Read tools.

use super::*;

impl Server {
    pub(super) fn list_cells(&self) -> Value {
        let mut cells: Vec<&Cell> = self.engine.all_cells().collect();
        cells.sort_by(|a, b| a.name.cmp(&b.name));
        json!(cells
            .iter()
            .map(|c| json!({
                "name": c.name,
                "kind": c.kind,
                "value": c.value,
                "state": c.state,
                "mark": c.mark.as_ref().map(|m| m.kind),
                "provenance": c.provenance,
            }))
            .collect::<Vec<_>>())
    }

    pub(super) fn get_cell(&self, name: &str) -> std::result::Result<Value, String> {
        let Some(cell) = self.engine.cell(name) else {
            return Err(format!("no such cell: {name}"));
        };
        Ok(json!({
            "name": cell.name,
            "kind": cell.kind,
            "value": cell.value,
            "source": cell.source,
            "state": cell.state,
            "exit_code": cell.exit_code,
            "stderr": cell.stderr,
            "duration_ms": cell.duration_ms,
            "deps": cell.deps,
            "dependents": self.engine.dependents_of(name),
            "provenance": cell.provenance,
            "decision": cell.decision,
            "pending_suggestions": cell.pending_suggestions,
            "mark": cell.mark,
            "cite": cell.cite,
            "answer_type": cell.answer_type,
            "usage": cell.usage,
            "session_id": cell.session_id,
        }))
    }
}

/// The member names of a `from`/`to` (or `from`/`values`) range argument.
pub(super) fn range_names(args: &Value) -> Vec<String> {
    if let Some(names) = args["names"].as_array() {
        return names
            .iter()
            .filter_map(|n| n.as_str().map(str::to_string))
            .collect();
    }
    if let Some(cells) = args["cells"].as_object() {
        return cells.keys().cloned().collect();
    }
    let from = args["from"].as_str().unwrap_or_default();
    let Some((sheet, c1, r1)) = cell_coord(from) else {
        return Vec::new();
    };
    // The block is on `from`'s sheet; `to` only says where it ends.
    let (c2, r2) = match args["to"].as_str().and_then(cell_coord) {
        Some((_, c, r)) => (c, r),
        None => match args["values"].as_array() {
            Some(rows) => {
                let width = rows
                    .iter()
                    .filter_map(|r| r.as_array())
                    .map(|r| r.len())
                    .max()
                    .unwrap_or(0) as u32;
                if width == 0 {
                    return Vec::new();
                }
                (c1 + width - 1, r1 + rows.len() as u32 - 1)
            }
            None => return Vec::new(),
        },
    };
    crate::model::range_cells(&on_sheet(sheet, &a1_name(c1, r1)), &a1_name(c2, r2))
        .unwrap_or_default()
}

pub(super) fn get_range(engine: &Engine, args: &Value) -> std::result::Result<Value, String> {
    if let Some(names) = args["names"].as_array() {
        // Any cells by name, named or A1: {name: value}; a missing one is "".
        let mut out = serde_json::Map::new();
        for n in names {
            let n = n.as_str().ok_or("names must be strings")?;
            out.insert(
                n.to_string(),
                json!(engine.cell(n).map(|c| c.value.clone()).unwrap_or_default()),
            );
        }
        return Ok(Value::Object(out));
    }
    let from = args["from"].as_str().unwrap_or_default();
    let to = args["to"].as_str().unwrap_or_default();
    let corner = |c: &'_ str| {
        cell_coord(c).map(|(s, col, row)| (s.map(str::to_string), col, row)).ok_or_else(|| {
            if c.is_empty() {
                "get_range needs from and to (an A1 block), or names: [...]".to_string()
            } else {
                format!("{c:?} isn't an A1 cell, so it can't be a corner; to read named cells in one call, pass names: [\"{c}\", …]")
            }
        })
    };
    let (sheet, c1, r1) = corner(from)?;
    let (_, c2, r2) = corner(to)?;
    let (lo_c, hi_c) = (c1.min(c2), c1.max(c2));
    let (lo_r, hi_r) = (r1.min(r2), r1.max(r2));
    let mut rows = Vec::new();
    for r in lo_r..=hi_r {
        let mut row = Vec::new();
        for c in lo_c..=hi_c {
            row.push(
                engine
                    .cell(&on_sheet(sheet.as_deref(), &a1_name(c, r)))
                    .map(|c| c.value.clone())
                    .unwrap_or_default(),
            );
        }
        rows.push(row);
    }
    Ok(json!(rows))
}
