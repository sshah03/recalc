//! The native menu bar. Item ids are the webview's action ids. Shortcuts are
//! declared only here: the OS delivers them to the menu before the webview.

use std::collections::HashMap;
use std::sync::Mutex;
use tauri::menu::{
    Menu, MenuBuilder, MenuItem, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder,
};
use tauri::{AppHandle, Manager, Wry};

/// Handles to the items whose enabled state follows the selection.
#[derive(Default)]
pub struct Items(Mutex<HashMap<String, MenuItem<Wry>>>);

impl Items {
    pub fn set_enabled(&self, id: &str, on: bool) {
        if let Some(item) = self.0.lock().unwrap().get(id) {
            let _ = item.set_enabled(on);
        }
    }
}

/// (id, label, accelerator) per menu, in order. An empty id is a separator.
type Spec = &'static [(&'static str, &'static str, &'static str)];

const FILE: Spec = &[
    ("file.new_board", "New Board…", "CmdOrCtrl+N"),
    ("file.open", "Open…", "CmdOrCtrl+O"),
    ("file.open_board", "Open Folder…", ""),
    ("file.open_recent", "Open Recent…", ""),
    ("", "", ""),
    ("file.template", "Insert Template…", ""),
    ("file.import_csv", "Import CSV…", ""),
    ("file.import_files", "Import Files as Rows…", ""),
    ("", "", ""),
    ("file.save", "Save", "CmdOrCtrl+S"),
    ("file.save_copy", "Save a Copy…", "CmdOrCtrl+Shift+S"),
    ("file.rename", "Rename…", ""),
    ("file.move", "Move To…", ""),
    ("file.save_template", "Save Board as Template…", ""),
    ("", "", ""),
    ("file.export_board", "Export Board…", ""),
    ("file.export_csv", "Export CSV…", ""),
    ("file.export_xlsx", "Export Excel Workbook…", ""),
];

const EDIT_TOP: Spec = &[
    ("edit.undo", "Undo", "CmdOrCtrl+Z"),
    ("edit.redo", "Redo", "CmdOrCtrl+Shift+Z"),
];

/// Select All is the board's own: on the grid it selects the filled cells, and in a
/// text field the page selects its text.
const EDIT_SELECT: Spec = &[("edit.select_all", "Select All", "CmdOrCtrl+A")];

const EDIT_BOTTOM: Spec = &[
    ("edit.paste_values", "Paste Values", "CmdOrCtrl+Shift+V"),
    (
        "edit.copy_markdown",
        "Copy as Markdown Table",
        "CmdOrCtrl+Shift+C",
    ),
    ("edit.delete", "Delete Cells", ""),
    ("", "", ""),
    ("edit.fill_down", "Fill Down", "CmdOrCtrl+D"),
    ("edit.fill_right", "Fill Right", "CmdOrCtrl+R"),
    ("", "", ""),
    ("edit.insert_row", "Insert Row Above", ""),
    ("edit.insert_row_below", "Insert Row Below", ""),
    ("edit.insert_col", "Insert Column Left", ""),
    ("edit.insert_col_right", "Insert Column Right", ""),
    ("edit.delete_rows", "Delete Rows", ""),
    ("edit.delete_cols", "Delete Columns", ""),
    ("", "", ""),
    ("edit.sort_asc", "Sort Rows A → Z", ""),
    ("edit.sort_desc", "Sort Rows Z → A", ""),
    ("", "", ""),
    ("edit.find", "Find…", "CmdOrCtrl+F"),
    ("edit.new_cell", "New Named Cell…", "Alt+CmdOrCtrl+N"),
];

const FORMAT: Spec = &[
    ("format.bold", "Bold", "CmdOrCtrl+B"),
    ("format.italic", "Italic", "CmdOrCtrl+I"),
    ("", "", ""),
    ("format.align_left", "Align Left", "CmdOrCtrl+Shift+L"),
    ("format.align_center", "Align Center", "CmdOrCtrl+Shift+E"),
    ("format.align_right", "Align Right", "CmdOrCtrl+Shift+R"),
    ("", "", ""),
    ("format.num_auto", "Automatic", ""),
    ("format.num_text", "Plain Text", ""),
    ("format.num_number", "Number", ""),
    ("format.num_integer", "Integer", ""),
    ("format.num_currency", "Currency", ""),
    ("format.num_percent", "Percent", ""),
    ("format.num_date", "Date", ""),
    ("", "", ""),
    ("format.fill", "Fill Color…", ""),
    ("format.header", "Make Header Row", ""),
    ("format.more", "Format Cells…", "CmdOrCtrl+1"),
    ("format.number", "Number Format…", ""),
    ("", "", ""),
    ("format.clear", "Clear Formatting", "CmdOrCtrl+Backslash"),
];

const VIEW: Spec = &[
    ("view.wrap", "Wrap Text", "Alt+CmdOrCtrl+W"),
    ("view.freeze", "Freeze Panes at Selection", ""),
    ("view.unfreeze", "Unfreeze Panes", ""),
    ("view.source", "Show Source", "Alt+CmdOrCtrl+S"),
    ("view.notify", "Notify When Something Needs You", ""),
    ("", "", ""),
    ("view.filter_all", "Show All Rows", ""),
    ("view.filter_flagged", "Show Flagged Rows", ""),
    ("view.filter_unverified", "Show Unverified Rows", ""),
    ("view.filter_errors", "Show Rows with Errors", ""),
    ("view.filter_running", "Show Running Rows", ""),
    ("view.filter_value", "Filter to This Value", ""),
    ("view.filter_column", "Filter Column…", ""),
];

const CELL: Spec = &[
    ("cell.edit", "Edit Cell", ""),
    ("cell.run", "Run", "CmdOrCtrl+Enter"),
    (
        "cell.run_selection",
        "Run Selection",
        "CmdOrCtrl+Shift+Enter",
    ),
    ("cell.run_column", "Run Column", "Alt+CmdOrCtrl+Enter"),
    ("cell.driver_answer", "Ask the Driver to Answer", ""),
    ("cell.stop", "Stop", "CmdOrCtrl+."),
    ("cell.stop_all", "Stop All Runs", "CmdOrCtrl+Shift+."),
    ("cell.terminal", "Open Terminal", "CmdOrCtrl+T"),
    ("", "", ""),
    ("cell.verify", "Verify / Unverify", "CmdOrCtrl+Y"),
    ("cell.flag", "Flag…", "CmdOrCtrl+Shift+Y"),
    (
        "cell.next_unverified",
        "Next Unverified Answer",
        "Alt+CmdOrCtrl+Down",
    ),
    ("cell.verify_next", "Verify and Next", "Alt+CmdOrCtrl+Y"),
    ("cell.review", "Review Answers", "Alt+CmdOrCtrl+R"),
    ("", "", ""),
    (
        "cell.add_column",
        "Add Question Column…",
        "CmdOrCtrl+Shift+Q",
    ),
    ("cell.answer_type", "Answer Type…", ""),
    ("cell.sparkline", "Show / Hide History Chart", ""),
];

const AGENT: Spec = &[
    ("agent.new", "New Agent Cell", "CmdOrCtrl+Shift+A"),
    ("agent.session", "Open Session", "CmdOrCtrl+Shift+T"),
    ("agent.terminal", "Open in Terminal", "Alt+CmdOrCtrl+T"),
    ("agent.notify", "Tell Agent About Changed Inputs", ""),
    ("agent.end", "End Session", ""),
    ("", "", ""),
    ("agent.driver", "Driver", "CmdOrCtrl+Shift+D"),
    ("agent.wide_driver", "Wide Driver", "Alt+CmdOrCtrl+D"),
    ("agent.decisions", "Needs You", "CmdOrCtrl+J"),
    ("cell.budget", "Set Agent Budget…", ""),
    ("agent.connect", "Connect an Agent…", ""),
];

const HELP: Spec = &[
    ("help.palette", "Command Palette", "CmdOrCtrl+K"),
    ("help.cheatsheet", "Cheatsheet", "CmdOrCtrl+/"),
    ("help.review", "Document Grids Guide", ""),
    ("help.welcome", "Welcome Tour", ""),
    ("help.report", "Report a Problem…", ""),
];

/// The app's own items: in the recalc menu on macOS, at the end of Help elsewhere.
const APP: Spec = &[
    ("app.updates", "Check for Updates…", ""),
    ("app.cli", "Install Command Line Tool…", ""),
];

/// Every spec, for the test that ties menu ids to the webview's actions.
#[cfg(test)]
const ALL: &[Spec] = &[
    FILE,
    EDIT_TOP,
    EDIT_SELECT,
    EDIT_BOTTOM,
    FORMAT,
    VIEW,
    CELL,
    AGENT,
    HELP,
    APP,
];

fn add_items<'a>(
    mut sub: SubmenuBuilder<'a, Wry, AppHandle>,
    app: &AppHandle,
    spec: Spec,
    items: &mut HashMap<String, MenuItem<Wry>>,
) -> tauri::Result<SubmenuBuilder<'a, Wry, AppHandle>> {
    for (id, label, accel) in spec {
        if id.is_empty() {
            sub = sub.separator();
            continue;
        }
        let mut b = MenuItemBuilder::with_id(*id, *label);
        if !accel.is_empty() {
            b = b.accelerator(accel);
        }
        let item = b.build(app)?;
        sub = sub.item(&item);
        items.insert(id.to_string(), item);
    }
    Ok(sub)
}

pub fn build(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let mut items = HashMap::new();
    // Mutated only by the macOS app menu.
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut menu = MenuBuilder::new(app);

    #[cfg(target_os = "macos")]
    {
        let about = tauri::menu::AboutMetadataBuilder::new()
            .name(Some("recalc"))
            .version(Some(concat!(env!("CARGO_PKG_VERSION"), " beta")))
            .comments(Some("A reactive grid you and your coding agents share."))
            .build();
        let app_menu = SubmenuBuilder::new(app, "recalc").item(&PredefinedMenuItem::about(
            app,
            None,
            Some(about),
        )?);
        let app_menu = add_items(app_menu, app, APP, &mut items)?
            .separator()
            .item(&PredefinedMenuItem::services(app, None)?)
            .separator()
            .item(&PredefinedMenuItem::hide(app, None)?)
            .item(&PredefinedMenuItem::hide_others(app, None)?)
            .item(&PredefinedMenuItem::show_all(app, None)?)
            .separator()
            .item(&PredefinedMenuItem::quit(app, None)?)
            .build()?;
        menu = menu.item(&app_menu);
    }

    let mut file = add_items(SubmenuBuilder::new(app, "File"), app, FILE, &mut items)?;
    file = file
        .separator()
        .item(&PredefinedMenuItem::close_window(app, None)?);
    #[cfg(not(target_os = "macos"))]
    {
        file = file.item(&PredefinedMenuItem::quit(app, None)?);
    }
    let file = file.build()?;

    let edit = add_items(SubmenuBuilder::new(app, "Edit"), app, EDIT_TOP, &mut items)?
        .separator()
        .cut()
        .copy()
        .paste();
    let edit = add_items(edit, app, EDIT_SELECT, &mut items)?.separator();
    let edit = add_items(edit, app, EDIT_BOTTOM, &mut items)?.build()?;

    let format = add_items(SubmenuBuilder::new(app, "Format"), app, FORMAT, &mut items)?.build()?;
    let view = add_items(SubmenuBuilder::new(app, "View"), app, VIEW, &mut items)?.build()?;
    let cell = add_items(SubmenuBuilder::new(app, "Cell"), app, CELL, &mut items)?.build()?;
    let agent = add_items(SubmenuBuilder::new(app, "Agent"), app, AGENT, &mut items)?.build()?;
    let help = add_items(SubmenuBuilder::new(app, "Help"), app, HELP, &mut items)?;
    #[cfg(not(target_os = "macos"))]
    let help = add_items(help.separator(), app, APP, &mut items)?;
    let help = help.build()?;

    app.manage(Items(Mutex::new(items)));
    menu.items(&[&file, &edit, &format, &view, &cell, &agent, &help])
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A menu item whose id the webview doesn't know is a dead menu item.
    #[test]
    fn every_menu_id_is_an_action() {
        let actions = include_str!("../../frontend/src/actions.ts");
        for spec in ALL {
            for (id, _, _) in spec.iter().filter(|(id, _, _)| !id.is_empty()) {
                assert!(
                    actions.contains(&format!("id: \"{id}\"")),
                    "menu id {id} has no action in actions.ts"
                );
            }
        }
    }
}
