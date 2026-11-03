//! Rows from a command: a `$` cell annotated `# rows: A2 key=number cols=number,title`
//! keeps one row per record of its output, matched by key. A record already on
//! the board updates its row's fields and leaves the other columns (answers,
//! marks) alone; a new one is appended, with the formulas, commands and agents
//! of the row above filled into it; one that's gone stays, marked.

use super::*;

/// A parsed `# rows:` annotation.
#[derive(Debug, PartialEq)]
pub(super) struct RowsSpec {
    col: u32,
    row: u32,
    /// The field rows are matched by; `None` for line output, where the line is the key.
    key: Option<String>,
    /// Fields written left to right from the anchor's column.
    cols: Vec<String>,
    /// A column that reads `gone` on rows whose record has left the output.
    gone: Option<u32>,
}

pub(super) fn parse_rows_spec(spec: &str) -> Result<RowsSpec, String> {
    let mut words = spec.split_whitespace();
    let anchor = words.next().unwrap_or_default();
    let (col, row) = a1_coord(anchor).ok_or_else(|| {
        format!("rows: {anchor:?} isn't a cell; start with where the rows go, like A2")
    })?;
    let mut out = RowsSpec {
        col,
        row,
        key: None,
        cols: Vec::new(),
        gone: None,
    };
    for word in words {
        let (k, v) = word
            .split_once('=')
            .ok_or_else(|| format!("rows: {word:?} should be key=, cols= or gone="))?;
        match k {
            "key" => out.key = Some(v.to_string()),
            "cols" => {
                out.cols = v
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect()
            }
            "gone" => {
                let (c, _) = a1_coord(&format!("{v}1"))
                    .ok_or_else(|| format!("rows: gone={v} should be a column letter"))?;
                out.gone = Some(c);
            }
            _ => {
                return Err(format!(
                    "rows: unknown setting {k:?}; use key=, cols= or gone="
                ))
            }
        }
    }
    if out.cols.is_empty() {
        if let Some(k) = &out.key {
            out.cols.push(k.clone());
        }
    } else if out.key.is_none() {
        out.key = out.cols.first().cloned();
    }
    if let Some(k) = &out.key {
        if !out.cols.contains(k) {
            return Err(format!("rows: key {k:?} must be one of cols"));
        }
    }
    if let Some(g) = out.gone {
        if g >= col && g < col + out.cols.len().max(1) as u32 {
            return Err("rows: the gone column can't be one of the field columns".into());
        }
    }
    Ok(out)
}

/// The output as (key, fields) records. JSON: an array of objects, or an
/// object of objects (each one's property name is the field `_key`). Anything
/// else: one record per non-empty line.
fn records(output: &str, spec: &RowsSpec) -> Result<Vec<(String, Vec<String>)>, String> {
    let Some(key) = &spec.key else {
        return Ok(output
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(|l| (l.to_string(), vec![l.to_string()]))
            .collect());
    };
    let json: serde_json::Value = serde_json::from_str(output.trim()).map_err(|e| {
        format!("rows: the output isn't JSON ({e}); without key= each line is a row")
    })?;
    let objects: Vec<serde_json::Value> = match json {
        serde_json::Value::Array(items) => items,
        serde_json::Value::Object(map) => map
            .into_iter()
            .map(|(name, mut v)| {
                if let Some(o) = v.as_object_mut() {
                    o.insert("_key".into(), serde_json::Value::String(name));
                }
                v
            })
            .collect(),
        _ => {
            return Err(
                "rows: the JSON should be an array of objects, or an object of objects".into(),
            )
        }
    };
    let field = |o: &serde_json::Value, path: &str| -> String {
        let v = path.split('.').try_fold(o, |v, part| v.get(part));
        match v {
            None | Some(serde_json::Value::Null) => String::new(),
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
        }
    };
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for o in &objects {
        let k = field(o, key);
        if k.is_empty() || !seen.insert(k.clone()) {
            continue; // no key, or a repeat: the first one wins
        }
        out.push((k, spec.cols.iter().map(|c| field(o, c)).collect()));
    }
    Ok(out)
}

impl Engine {
    /// After `name` ran cleanly, bring its rows in line with its output.
    pub(super) fn sync_rows(&mut self, name: &str) -> Result<()> {
        let Some(cell) = self.cells.get(name) else {
            return Ok(());
        };
        if cell.kind != CellKind::Command
            || cell.state != CellState::Ok
            || cell.exit_code != Some(0)
        {
            return Ok(());
        }
        let source = cell.source.clone().unwrap_or_default();
        let (_, ann) = split_annotations(def_body(&source));
        let Some(spec) = ann.rows else { return Ok(()) };
        let spec = match parse_rows_spec(spec) {
            Ok(s) => s,
            Err(e) => {
                self.log(format!("{name}: {e}"));
                return Ok(());
            }
        };
        let recs = match records(&cell.value, &spec) {
            Ok(r) => r,
            Err(e) => {
                self.log(format!("{name}: {e}"));
                return Ok(());
            }
        };
        let width = spec.cols.len().max(1) as u32;
        // The rows go on the command's own sheet.
        let sheet = split_sheet(name).0.map(str::to_string);
        let cell_at = |c: u32, r: u32| on_sheet(sheet.as_deref(), &a1_name(c, r));
        let key_col = spec.col
            + spec
                .key
                .as_ref()
                .and_then(|k| spec.cols.iter().position(|c| c == k))
                .unwrap_or(0) as u32;
        // The rows already there: down the key column until a blank.
        let mut at: HashMap<String, u32> = HashMap::new();
        let mut last = spec.row - 1;
        while let Some(v) = self
            .cells
            .get(&cell_at(key_col, last + 1))
            .map(|c| c.value.clone())
        {
            if v.is_empty() {
                break;
            }
            last += 1;
            at.entry(v).or_insert(last);
        }
        let mut writes = Vec::new();
        let mut added = Vec::new();
        let mut present = HashSet::new();
        for (key, fields) in recs {
            let row = match at.get(&key) {
                Some(&r) => r,
                None => {
                    last += 1;
                    added.push(last);
                    last
                }
            };
            present.insert(row);
            for (i, f) in fields.into_iter().enumerate() {
                writes.push((cell_at(spec.col + i as u32, row), f));
            }
            if let Some(g) = spec.gone {
                writes.push((cell_at(g, row), String::new()));
            }
        }
        let gone: Vec<u32> = at
            .values()
            .copied()
            .filter(|r| !present.contains(r))
            .collect();
        if let Some(g) = spec.gone {
            for &r in &gone {
                writes.push((cell_at(g, r), "gone".into()));
            }
        }
        self.write_literals(writes, Actor::Engine)?;
        // A new row takes the definitions of the row above it, past the fields.
        for &row in &added {
            if row <= spec.row {
                continue;
            }
            // Across the whole row above: a blank cell in it doesn't end the row.
            let width_above = self
                .cells
                .keys()
                .filter_map(|n| cell_coord(n))
                .filter(|&(s, _, r)| s == sheet.as_deref() && r == row - 1)
                .map(|(_, c, _)| c)
                .max()
                .unwrap_or(0);
            for c in 1..=width_above {
                let Some(above) = self.cells.get(&cell_at(c, row - 1)) else {
                    continue;
                };
                let in_fields = c >= spec.col && c < spec.col + width;
                if !in_fields && Some(c) != spec.gone && above.source.is_some() {
                    let answer_type = above.answer_type.clone();
                    let target = cell_at(c, row);
                    self.fill(&target, &target, Fill::Down, Actor::Engine)?;
                    // The column's answer type comes down with its prompt.
                    if answer_type.is_some()
                        && self
                            .cells
                            .get(&target)
                            .is_some_and(|t| t.answer_type.is_none())
                    {
                        self.set_answer_type(&target, answer_type)?;
                    }
                }
            }
        }
        for row in (spec.row..=last).filter(|r| at.values().any(|v| v == r)) {
            let key_cell = cell_at(key_col, row);
            let mut format = self
                .cells
                .get(&key_cell)
                .and_then(|c| c.format.clone())
                .unwrap_or_default();
            let gray = Some(crate::model::Fill::Gray);
            let want = if gone.contains(&row) {
                gray
            } else if format.fill == gray {
                None
            } else {
                format.fill
            };
            if format.fill != want {
                format.fill = want;
                self.set_format(&key_cell, (format != Format::default()).then_some(format))?;
            }
        }
        if !added.is_empty() || !gone.is_empty() {
            self.log(format!(
                "{name}: {} new row{}, {} gone",
                added.len(),
                if added.len() == 1 { "" } else { "s" },
                gone.len()
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod spec_tests {
    use super::*;

    #[test]
    fn a_rows_annotation_names_its_anchor_key_and_columns() {
        let s = parse_rows_spec("A2 key=number cols=number,title,author.login gone=F").unwrap();
        assert_eq!((s.col, s.row, s.gone), (1, 2, Some(6)));
        assert_eq!(s.key.as_deref(), Some("number"));
        assert_eq!(s.cols, ["number", "title", "author.login"]);
        // cols alone keys on the first; neither means lines.
        assert_eq!(
            parse_rows_spec("B3 cols=id,name").unwrap().key.as_deref(),
            Some("id")
        );
        assert_eq!(parse_rows_spec("A2").unwrap().key, None);
        assert!(parse_rows_spec("here")
            .unwrap_err()
            .contains("isn't a cell"));
        assert!(parse_rows_spec("A2 key=x cols=a,b")
            .unwrap_err()
            .contains("one of cols"));
        assert!(parse_rows_spec("A2 cols=a,b gone=B")
            .unwrap_err()
            .contains("field columns"));
    }

    #[test]
    fn output_becomes_records_from_json_arrays_maps_or_lines() {
        let spec = parse_rows_spec("A2 key=n cols=n,user.name").unwrap();
        let r = records(r#"[{"n":1,"user":{"name":"ada"}},{"n":2},{"n":1}]"#, &spec).unwrap();
        assert_eq!(
            r,
            [
                ("1".into(), vec!["1".into(), "ada".into()]),
                ("2".into(), vec!["2".into(), String::new()])
            ]
        );
        let spec = parse_rows_spec("A2 key=_key cols=_key,latest").unwrap();
        let r = records(r#"{"left-pad":{"latest":"1.3.0"}}"#, &spec).unwrap();
        assert_eq!(
            r,
            [("left-pad".into(), vec!["left-pad".into(), "1.3.0".into()])]
        );
        let spec = parse_rows_spec("A2").unwrap();
        assert_eq!(records("a.rs\n\nb.rs\n", &spec).unwrap().len(), 2);
        assert!(records("not json", &parse_rows_spec("A2 key=n").unwrap()).is_err());
    }
}
