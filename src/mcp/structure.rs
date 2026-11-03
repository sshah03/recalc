//! Tools that reshape the board: fill, insert and delete, sort, stop, format.

use super::*;
use crate::engine::{Axis, Fill};
use crate::model::{a1_coord, cell_coord, Format};

impl Server {
    /// A cell's own agent computes one cell; reshaping the board is the
    /// driver's or a person's call.
    fn refuse_for_cell_agent(&self, tool: &str) -> std::result::Result<(), String> {
        if self.cell_agent {
            return Err(format!(
                "an agent computing one cell can't {tool}; put what you need in your answer"
            ));
        }
        Ok(())
    }

    pub(super) fn fill_range(
        &mut self,
        args: &Value,
        by: Actor,
    ) -> std::result::Result<Value, String> {
        self.refuse_for_cell_agent("fill")?;
        let from = args["from"].as_str().unwrap_or_default();
        let to = args["to"].as_str().unwrap_or_default();
        let one_row = matches!(
            (cell_coord(from), cell_coord(to)),
            (Some((_, _, a)), Some((_, _, b))) if a == b
        );
        let dir = match args["direction"].as_str() {
            Some("down") => Fill::Down,
            Some("right") => Fill::Right,
            None if one_row => Fill::Right,
            None => Fill::Down,
            Some(other) => return Err(format!("direction is down or right, not {other:?}")),
        };
        let written = self
            .engine
            .fill(from, to, dir, by)
            .map_err(|e| e.to_string())?;
        Ok(json!({"ok": true, "filled": written.len()}))
    }

    pub(super) fn edit_structure(
        &mut self,
        args: &Value,
        by: Actor,
    ) -> std::result::Result<Value, String> {
        self.refuse_for_cell_agent("insert or delete rows or columns")?;
        let axis = match args["axis"].as_str() {
            Some("rows") => Axis::Rows,
            Some("columns") => Axis::Cols,
            _ => return Err("axis is rows or columns".into()),
        };
        let at = args["at"]
            .as_u64()
            .ok_or("at is a 1-based row or column number")? as u32;
        let count = args["count"].as_u64().unwrap_or(1) as u32;
        let sheet = self
            .engine
            .sheet_arg(args["sheet"].as_str())
            .map_err(|e| e.to_string())?;
        match args["action"].as_str() {
            Some("insert") => self
                .engine
                .insert(sheet.as_deref(), axis, at, count, by)
                .map_err(|e| e.to_string())?,
            Some("delete") => {
                self.engine
                    .delete(sheet.as_deref(), axis, at, count, by)
                    .map_err(|e| e.to_string())?;
            }
            _ => return Err("action is insert or delete".into()),
        }
        Ok(json!({"ok": true}))
    }

    pub(super) fn sort_rows(
        &mut self,
        args: &Value,
        by: Actor,
    ) -> std::result::Result<Value, String> {
        self.refuse_for_cell_agent("sort")?;
        let column = args["column"].as_str().unwrap_or_default();
        let (col, _) = a1_coord(&format!("{column}1"))
            .ok_or_else(|| format!("column is a letter such as C, not {column:?}"))?;
        let first = args["first"]
            .as_u64()
            .ok_or("first is a 1-based row number")? as u32;
        let last = args["last"]
            .as_u64()
            .ok_or("last is a 1-based row number")? as u32;
        let descending = args["descending"].as_bool().unwrap_or(false);
        let sheet = self
            .engine
            .sheet_arg(args["sheet"].as_str())
            .map_err(|e| e.to_string())?;
        self.engine
            .sort_rows(sheet.as_deref(), col, first, last, descending, by)
            .map_err(|e| e.to_string())?;
        Ok(json!({"ok": true}))
    }

    pub(super) fn rename_sheet(&mut self, args: &Value) -> std::result::Result<Value, String> {
        self.refuse_for_cell_agent("rename a sheet")?;
        let from = args["from"].as_str().unwrap_or_default().trim();
        let to = args["to"].as_str().unwrap_or_default().trim();
        if from.is_empty() || to.is_empty() {
            return Err("from and to are sheet names, e.g. Sheet1 and Runs".into());
        }
        self.engine
            .rename_sheet(from, to)
            .map_err(|e| e.to_string())?;
        let sheets = self.engine.sheets();
        Ok(json!({"ok": true, "first": sheets.first, "others": sheets.others}))
    }

    pub(super) fn duplicate_sheet(
        &mut self,
        args: &Value,
        by: Actor,
    ) -> std::result::Result<Value, String> {
        self.refuse_for_cell_agent("duplicate a sheet")?;
        let from = args["from"].as_str().unwrap_or_default().trim();
        let to = args["to"].as_str().unwrap_or_default().trim();
        if from.is_empty() || to.is_empty() {
            return Err("from and to are sheet names, e.g. Harbor and Pinecrest".into());
        }
        self.engine
            .duplicate_sheet(from, to, by)
            .map_err(|e| e.to_string())?;
        let sheets = self.engine.sheets();
        Ok(json!({"ok": true, "first": sheets.first, "others": sheets.others}))
    }

    pub(super) fn stop_cell(
        &mut self,
        name: &str,
        by: Actor,
    ) -> std::result::Result<Value, String> {
        self.refuse_for_cell_agent("stop runs")?;
        let stopped = self.engine.stop(name, by).map_err(|e| e.to_string())?;
        Ok(json!({"ok": true, "stopped": stopped}))
    }

    pub(super) fn set_format(&mut self, args: &Value) -> std::result::Result<Value, String> {
        let names = answer_type_targets(args);
        if names.is_empty() {
            return Err("set_format needs name or range".into());
        }
        self.refuse_outside(&names, "format")?;
        let mut fields = serde_json::Map::new();
        for key in [
            "bold",
            "italic",
            "align",
            "num",
            "fill",
            "wrap",
            "decimals",
            "currency",
            "highlight",
        ] {
            if !args[key].is_null() {
                fields.insert(key.into(), args[key].clone());
            }
        }
        let format: Format =
            serde_json::from_value(Value::Object(fields)).map_err(|e| e.to_string())?;
        let format = (!format.is_plain()).then_some(format);
        for name in &names {
            self.engine
                .set_format(name, format.clone())
                .map_err(|e| e.to_string())?;
        }
        Ok(json!({"ok": true, "formatted": names.len()}))
    }
}
