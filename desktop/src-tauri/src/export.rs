//! Excel export: displayed values, numeric where a number preset applies,
//! with formatting; a citation or review mark becomes the cell's note.

use recalc::engine::Engine;
use recalc::model::{a1_name, cell_coord, Align, Cell, Fill, Format, MarkKind, NumFormat};
use rust_xlsxwriter::{Color, Format as XFormat, FormatAlign, Note, Workbook};
use std::path::Path;

/// The fills as the light theme paints them.
fn fill_color(fill: Fill) -> Color {
    match fill {
        Fill::Yellow => Color::RGB(0xFFF3BF),
        Fill::Green => Color::RGB(0xDCF3E4),
        Fill::Blue => Color::RGB(0xDDE9FB),
        Fill::Red => Color::RGB(0xFBE0E0),
        Fill::Purple => Color::RGB(0xEBE3FB),
        Fill::Gray => Color::RGB(0xECECEF),
    }
}

/// The symbol Excel shows for a currency code; others are written as the code.
fn currency_prefix(code: Option<&str>) -> String {
    let sym = match code.unwrap_or("USD") {
        "USD" => "$",
        "EUR" => "€",
        "GBP" => "£",
        "JPY" | "CNY" => "¥",
        "INR" => "₹",
        "KRW" => "₩",
        "CAD" => "CA$",
        "AUD" => "A$",
        other => return format!("\"{other} \""),
    };
    format!("\"{sym}\"")
}

/// `#,##0.00` with `decimals` places (two when unset).
fn places(decimals: Option<u8>) -> String {
    match decimals.unwrap_or(2) {
        0 => String::new(),
        d => format!(".{}", "0".repeat(d as usize)),
    }
}

/// A cell's format as Excel's, with the fill its highlights give `value` now.
fn xformat(f: &Format, value: &str) -> XFormat {
    let mut x = XFormat::new();
    if f.bold {
        x = x.set_bold();
    }
    if f.italic {
        x = x.set_italic();
    }
    x = match f.align {
        Some(Align::Center) => x.set_align(FormatAlign::Center),
        Some(Align::Right) => x.set_align(FormatAlign::Right),
        Some(Align::Left) => x.set_align(FormatAlign::Left),
        None => x,
    };
    x = match f.num {
        Some(NumFormat::Integer) => x.set_num_format("#,##0"),
        Some(NumFormat::Number) => x.set_num_format(format!("#,##0{}", places(f.decimals))),
        Some(NumFormat::Currency) => x.set_num_format(format!(
            "{}#,##0{}",
            currency_prefix(f.currency.as_deref()),
            places(f.decimals)
        )),
        Some(NumFormat::Percent) => x.set_num_format(format!("0{}%", places(f.decimals))),
        Some(NumFormat::Text | NumFormat::Date) | None => x,
    };
    if f.wrap {
        x = x.set_text_wrap();
    }
    if let Some(fill) = f.fill_for(value) {
        x = x.set_background_color(fill_color(fill));
    }
    x
}

/// The value as a number, when a number preset applies and it parses.
fn numeric(cell: &Cell) -> Option<f64> {
    let num = cell.format.as_ref()?.num?;
    if !matches!(
        num,
        NumFormat::Integer | NumFormat::Number | NumFormat::Currency | NumFormat::Percent
    ) {
        return None;
    }
    let cleaned: String = cell
        .value
        .trim()
        .chars()
        .filter(|c| !matches!(c, ',' | '$' | '%' | ' '))
        .collect();
    let n: f64 = cleaned.parse().ok()?;
    Some(if num == NumFormat::Percent {
        n / 100.0
    } else {
        n
    })
}

fn note_for(cell: &Cell) -> Option<String> {
    let mut lines = Vec::new();
    if let Some(m) = &cell.mark {
        let who = String::from(m.by.clone());
        lines.push(match (m.kind, &m.note) {
            (MarkKind::Verified, Some(n)) => format!("Verified by {who}: {n}"),
            (MarkKind::Verified, None) => format!("Verified by {who}"),
            (MarkKind::Flagged, Some(n)) => format!("Flagged by {who}: {n}"),
            (MarkKind::Flagged, None) => format!("Flagged by {who}"),
        });
    }
    if let Some(c) = &cell.cite {
        let mut src = format!("Source: {}", c.file);
        if let Some(w) = &c.location {
            src.push_str(&format!(", {w}"));
        }
        lines.push(src);
        lines.push(format!("\u{201c}{}\u{201d}", c.quote.trim()));
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

/// Column widths in characters from the layout's pixel widths, roughly.
fn widths(layout: &str) -> Vec<(u16, f64)> {
    let v: serde_json::Value = serde_json::from_str(layout).unwrap_or_default();
    v["widths"]
        .as_object()
        .map(|m| {
            m.iter()
                .filter_map(|(col, px)| {
                    let col: u16 = col.parse().ok()?;
                    Some((col.checked_sub(1)?, (px.as_f64()? / 7.0).clamp(4.0, 120.0)))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn frozen(layout: &str) -> (u32, u16) {
    let v: serde_json::Value = serde_json::from_str(layout).unwrap_or_default();
    (
        v["frozenRows"].as_u64().unwrap_or(0) as u32,
        v["frozenCols"].as_u64().unwrap_or(0) as u16,
    )
}

/// Write the board to `path` as .xlsx: a worksheet per sheet (the first
/// named after the board unless it was renamed), then the named cells
/// (summaries, counts) on one more.
pub fn write(engine: &Engine, layout: &str, path: &Path) -> Result<(), String> {
    let mut book = Workbook::new();
    let layout: serde_json::Value = serde_json::from_str(layout).unwrap_or_default();
    let sheets = engine.sheets();
    let first_name = if sheets.first == "Sheet1" {
        sheet_name(engine.dir())
    } else {
        sheets.first.clone()
    };
    let tabs = std::iter::once((None, first_name, layout.clone())).chain(
        sheets
            .others
            .iter()
            .map(|s| (Some(s.as_str()), s.clone(), layout["sheets"][s].clone())),
    );
    for (on, name, layout) in tabs {
        let sheet = book.add_worksheet();
        sheet.set_name(&name).map_err(|e| e.to_string())?;
        let mut cells: Vec<(u32, u16, &Cell)> = engine
            .all_cells()
            .filter_map(|c| {
                let (s, col, row) = cell_coord(&c.name)?;
                (s == on).then_some((row - 1, (col - 1) as u16, c))
            })
            .collect();
        cells.sort_by_key(|(r, c, _)| (*r, *c));
        for (row, col, cell) in cells {
            let fmt = cell.format.as_ref().map(|f| xformat(f, &cell.value));
            let r = match (numeric(cell), &fmt) {
                (Some(n), Some(f)) => sheet.write_number_with_format(row, col, n, f),
                (Some(n), None) => sheet.write_number(row, col, n),
                (None, Some(f)) => sheet.write_string_with_format(row, col, &cell.value, f),
                (None, None) => sheet.write_string(row, col, &cell.value),
            };
            r.map_err(|e| format!("{}: {e}", a1_name(col as u32 + 1, row + 1)))?;
            if let Some(text) = note_for(cell) {
                let note = Note::new(text).set_author("recalc");
                sheet
                    .insert_note(row, col, &note)
                    .map_err(|e| e.to_string())?;
            }
        }
        let layout = layout.to_string();
        for (col, w) in widths(&layout) {
            sheet.set_column_width(col, w).map_err(|e| e.to_string())?;
        }
        let (rows, cols) = frozen(&layout);
        if rows > 0 || cols > 0 {
            sheet
                .set_freeze_panes(rows, cols)
                .map_err(|e| e.to_string())?;
        }
    }
    let mut named: Vec<&Cell> = engine
        .all_cells()
        .filter(|c| cell_coord(&c.name).is_none())
        .collect();
    named.sort_by(|a, b| a.name.cmp(&b.name));
    if !named.is_empty() {
        let sheet = book.add_worksheet();
        sheet.set_name("Named cells").map_err(|e| e.to_string())?;
        let bold = XFormat::new().set_bold();
        for (col, head) in ["Name", "Value"].into_iter().enumerate() {
            sheet
                .write_string_with_format(0, col as u16, head, &bold)
                .map_err(|e| e.to_string())?;
        }
        for (i, cell) in named.into_iter().enumerate() {
            let row = i as u32 + 1;
            sheet
                .write_string(row, 0, &cell.name)
                .map_err(|e| e.to_string())?;
            let r = match numeric(cell).or_else(|| cell.value.trim().parse::<f64>().ok()) {
                Some(n) => sheet.write_number(row, 1, n),
                None => sheet.write_string(row, 1, &cell.value),
            };
            r.map_err(|e| format!("{}: {e}", cell.name))?;
        }
        sheet.set_column_width(0, 24).map_err(|e| e.to_string())?;
        sheet.set_column_width(1, 60).map_err(|e| e.to_string())?;
    }
    book.save(path).map_err(|e| e.to_string())
}

/// The board's folder name, as Excel allows a sheet to be named.
fn sheet_name(dir: &Path) -> String {
    let base = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    // A board package's name without its `.recalc`.
    let base = base.strip_suffix(".recalc").unwrap_or(&base).to_string();
    let clean: String = base
        .chars()
        .filter(|c| !matches!(c, '[' | ']' | ':' | '*' | '?' | '/' | '\\'))
        .take(31)
        .collect();
    let clean = clean.trim_matches('\'').trim().to_string();
    if clean.is_empty() {
        "Board".into()
    } else {
        clean
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use recalc::model::{Actor, Cite, Mark};

    #[tokio::test]
    async fn workbook_carries_values_formats_and_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = Engine::open(dir.path()).unwrap();
        e.set("A1", "Document", Actor::Human).unwrap();
        e.set_format(
            "A1",
            Some(Format {
                bold: true,
                ..Format::default()
            }),
        )
        .unwrap();
        e.set("B2", "1,250.5", Actor::Human).unwrap();
        e.set_format(
            "B2",
            Some(Format {
                num: Some(NumFormat::Currency),
                ..Format::default()
            }),
        )
        .unwrap();
        e.set("C2", "Consent required", Actor::Agent("a1".into()))
            .unwrap();
        e.set_cite(
            "C2",
            Some(Cite {
                file: "contracts/msa.md".into(),
                quote: "may not assign".into(),
                location: Some("§12".into()),
                by: Actor::Agent("a1".into()),
                at: 1,
                found: None,
            }),
        )
        .unwrap();
        e.set_mark(
            "C2",
            Some(Mark {
                kind: MarkKind::Verified,
                note: None,
                by: Actor::Human,
                at: 2,
            }),
        )
        .unwrap();
        e.set("total", "= 1", Actor::Human).unwrap();
        e.set("My Notes!B3", "never edit tests", Actor::Human)
            .unwrap();
        let out = dir.path().join("board.xlsx");
        write(
            &e,
            r#"{"widths":{"1":280},"frozenRows":1,"frozenCols":1,"sheets":{"My Notes":{"widths":{"2":420}}}}"#,
            &out,
        )
        .unwrap();
        // Notes live in the comments part of the zip.
        let file = std::fs::File::open(&out).unwrap();
        let mut zip = zip::ZipArchive::new(file).unwrap();
        let mut sheet = String::new();
        std::io::Read::read_to_string(
            &mut zip.by_name("xl/worksheets/sheet1.xml").unwrap(),
            &mut sheet,
        )
        .unwrap();
        assert!(
            sheet.contains("1250.5"),
            "currency cell is a number: {sheet}"
        );
        assert!(sheet.contains("<pane"), "panes frozen");
        let mut comments = String::new();
        std::io::Read::read_to_string(&mut zip.by_name("xl/comments1.xml").unwrap(), &mut comments)
            .unwrap();
        assert!(comments.contains("Verified by human"));
        assert!(comments.contains("Source: contracts/msa.md, §12"));
        assert!(comments.contains("may not assign"));
        // The grid's sheet is named after the board; named cells get their own.
        let mut book = String::new();
        std::io::Read::read_to_string(&mut zip.by_name("xl/workbook.xml").unwrap(), &mut book)
            .unwrap();
        assert!(book.contains("name=\"Named cells\""), "{book}");
        // Each sheet is a worksheet of its own, in order, with its own widths.
        let order: Vec<usize> = ["name=\"My Notes\"", "name=\"Named cells\""]
            .iter()
            .map(|n| book.find(n).unwrap_or_else(|| panic!("{n} in {book}")))
            .collect();
        assert!(order[0] < order[1], "{book}");
        let mut notes = String::new();
        std::io::Read::read_to_string(
            &mut zip.by_name("xl/worksheets/sheet2.xml").unwrap(),
            &mut notes,
        )
        .unwrap();
        assert!(notes.contains("r=\"B3\""), "{notes}");
        assert!(
            notes.contains("width=\"60"),
            "its own column width: {notes}"
        );
        assert!(!book.contains("name=\"Review\""), "{book}");
        let mut strings = String::new();
        std::io::Read::read_to_string(
            &mut zip.by_name("xl/sharedStrings.xml").unwrap(),
            &mut strings,
        )
        .unwrap();
        assert!(strings.contains(">total<"), "{strings}");
        assert_eq!(
            numeric(&Cell {
                value: "12.5%".into(),
                format: Some(Format {
                    num: Some(NumFormat::Percent),
                    ..Format::default()
                }),
                ..Cell::new("x")
            }),
            Some(0.125)
        );
    }

    #[tokio::test]
    async fn the_workbook_keeps_decimals_currencies_wrap_and_the_fill_a_highlight_gives() {
        use recalc::model::{Fill, Highlight, When};
        let dir = tempfile::tempdir().unwrap();
        let mut e = Engine::open(dir.path()).unwrap();
        e.set("B2", "1250.5", Actor::Human).unwrap();
        e.set_format(
            "B2",
            Some(Format {
                num: Some(NumFormat::Currency),
                currency: Some("EUR".into()),
                decimals: Some(3),
                ..Format::default()
            }),
        )
        .unwrap();
        let status = Format {
            wrap: true,
            highlight: vec![Highlight {
                when: When::Is,
                value: "fail".into(),
                fill: Fill::Red,
            }],
            ..Format::default()
        };
        e.set("C2", "FAIL", Actor::Human).unwrap();
        e.set_format("C2", Some(status.clone())).unwrap();
        e.set("C3", "pass", Actor::Human).unwrap();
        e.set_format("C3", Some(status)).unwrap();
        let out = dir.path().join("board.xlsx");
        write(&e, "{}", &out).unwrap();
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&out).unwrap()).unwrap();
        let mut styles = String::new();
        std::io::Read::read_to_string(&mut zip.by_name("xl/styles.xml").unwrap(), &mut styles)
            .unwrap();
        assert!(
            styles.contains("€") && styles.contains("#,##0.000"),
            "{styles}"
        );
        assert!(styles.contains("wrapText=\"1\""), "{styles}");
        // C2 matched its rule, C3 didn't: one red fill in the workbook.
        assert_eq!(styles.matches("FFFBE0E0").count(), 1, "{styles}");
    }
}
