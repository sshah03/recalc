//! Boards: opening, recents, import, export, the column library.

use super::*;

// ---- boards: open another directory, recents, import, export, share ----

#[tauri::command]
pub(crate) async fn recent_boards(state: State<'_, Shared>) -> Result<Vec<String>, String> {
    let current = state.lock().engine.dir().display().to_string();
    Ok(board::load_recents()
        .into_iter()
        .filter(|r| *r != current && std::path::Path::new(r).is_dir())
        .collect())
}

/// A new board: an empty "Untitled board" in the boards folder, open at
/// once, as ⌘N works in Numbers. Rename and Move put it elsewhere. A save
/// dialog here read as "save your changes?", and macOS opened it wherever
/// it last saved, even inside the open board.
#[tauri::command]
pub(crate) async fn new_board(
    app: AppHandle,
    state: State<'_, Shared>,
    ptys: State<'_, pty::Ptys>,
) -> Result<Option<String>, String> {
    let dir = fresh_folder(&boards_home(), "Untitled board")?;
    let mut next = Board::open(&dir)?;
    next.fresh = true;
    switch_to(&app, &state, &ptys, next, &dir);
    Ok(Some(dir.display().to_string()))
}

/// Where new boards go: `~/Documents/recalc boards`, or `$RECALC_BOARDS_DIR`.
pub(crate) fn boards_home() -> std::path::PathBuf {
    if let Some(dir) = std::env::var_os("RECALC_BOARDS_DIR") {
        return dir.into();
    }
    dirs::document_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| "/".into())
        .join("recalc boards")
}

/// Make an empty board folder named `stem` in `home`, numbered past the
/// names already taken ("Untitled board 2"), as a package on macOS.
pub(crate) fn fresh_folder(
    home: &std::path::Path,
    stem: &str,
) -> Result<std::path::PathBuf, String> {
    std::fs::create_dir_all(home).map_err(|e| format!("couldn't make {}: {e}", home.display()))?;
    for n in 1.. {
        let name = if n == 1 {
            stem.to_string()
        } else {
            format!("{stem} {n}")
        };
        let dir = as_package(home.join(&name));
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("couldn't make {}: {e}", dir.display())),
        }
    }
    unreachable!("some number is free")
}

/// Whether a board New Board made is still as it was made: no cells, and
/// nothing in its folder but recalc's own.
fn untouched(board: &Board) -> bool {
    board.fresh
        && board.engine.all_cells().next().is_none()
        && board.engine.dir().read_dir().is_ok_and(|entries| {
            entries
                .flatten()
                .all(|e| matches!(e.file_name().to_str(), Some(".recalc" | ".DS_Store")))
        })
}

/// File → Rename…: give the board's folder a new name where it is.
/// Returns the new folder.
#[tauri::command]
pub(crate) async fn rename_board(
    app: AppHandle,
    state: State<'_, Shared>,
    ptys: State<'_, pty::Ptys>,
    name: String,
) -> Result<String, String> {
    let name = name.trim().trim_end_matches(".recalc").trim();
    if name.is_empty() || name.contains('/') || name.contains(':') || name.starts_with('.') {
        return Err("A board's name can't be empty, start with a dot, or contain / or :".into());
    }
    let from = state.lock().engine.dir().to_path_buf();
    let parent = from.parent().ok_or("this board's folder has no parent")?;
    let to = if from.extension().is_some_and(|e| e == "recalc") {
        as_package(parent.join(name))
    } else {
        parent.join(name)
    };
    move_board_to(&app, &state, &ptys, &from, &to).map(|d| d.display().to_string())
}

/// File → Move To…: move the board's folder into a folder the person picks.
/// Returns the new folder, or None when the picker was cancelled.
#[tauri::command]
pub(crate) async fn move_board(
    app: AppHandle,
    state: State<'_, Shared>,
    ptys: State<'_, pty::Ptys>,
) -> Result<Option<String>, String> {
    let from = state.lock().engine.dir().to_path_buf();
    let Some(folder) = picked(|| {
        app.dialog()
            .file()
            .set_title("Move this board into a folder")
            .set_directory(from.parent().unwrap_or(&from))
            .blocking_pick_folder()
    }) else {
        return Ok(None);
    };
    let folder = folder.into_path().map_err(|e| e.to_string())?;
    let to = folder.join(from.file_name().ok_or("this board's folder has no name")?);
    move_board_to(&app, &state, &ptys, &from, &to).map(|d| Some(d.display().to_string()))
}

/// Move the open board's folder from `from` to `to` and reopen it there.
/// Refused while anything runs in it: a run or a session would lose its place.
fn move_board_to(
    app: &AppHandle,
    state: &Shared,
    ptys: &pty::Ptys,
    from: &std::path::Path,
    to: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    let shown = |p: &std::path::Path| {
        p.file_stem().map_or_else(
            || p.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        )
    };
    if to == from {
        return Ok(to.to_path_buf());
    }
    let real = |p: &std::path::Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    if real(to.parent().unwrap_or(to)).starts_with(real(from)) {
        return Err("A board can't move into its own folder.".into());
    }
    // On a disk that ignores case, "Q3" and "q3" are one folder: a rename, not a clash.
    if to.exists() && !same_folder(from, to) {
        return Err(format!(
            "There's already a “{}” in {}. Choose another name.",
            shown(to),
            to.parent()
                .map_or_else(String::new, |p| p.display().to_string())
        ));
    }
    {
        let mut board = state.lock();
        if let Some(why) = busy(&board) {
            return Err(why);
        }
        // Stop this engine before its folder moves: its claim on the board
        // (engine.pid) moves too, and would keep the reopened board passive.
        detach_all(&mut board, ptys);
        board.engine.shutdown();
    }
    let moved = std::fs::rename(from, to);
    let at = if moved.is_ok() { to } else { from };
    let next = Board::open(at)?;
    switch_to(app, state, ptys, next, at);
    moved.map_err(|e| {
        if e.raw_os_error() == Some(18) {
            "That's on another disk, and recalc only moves a board within one. Use Save a Copy… to put a copy there.".to_string()
        } else {
            format!("Couldn't move the board: {e}")
        }
    })?;
    Ok(to.to_path_buf())
}

fn same_folder(a: &std::path::Path, b: &std::path::Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (a.metadata(), b.metadata()) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

/// Why the open board can't move now, if it can't.
fn busy(board: &Board) -> Option<String> {
    use recalc::model::CellState;
    let engine = &board.engine;
    if engine
        .all_cells()
        .any(|c| matches!(c.state, CellState::Computing | CellState::Queued))
    {
        return Some(
            "Cells on this board are running. Stop them or let them finish, then try again.".into(),
        );
    }
    let dir = engine.dir();
    let live = engine
        .all_cells()
        .filter(|c| c.kind == CellKind::AgentCall)
        .map(|c| c.name.clone())
        .chain([session::DRIVER.to_string()])
        .any(|cell| session::alive(&session::name(dir, &cell)));
    live.then(|| "Agent sessions are open on this board, and moving it would lose their place. End them, then try again.".into())
}

/// A save dialog for a board's folder. On macOS a board of its own is a
/// `Name.recalc` package: Finder shows one document that opens in recalc, and
/// it's a folder underneath, so agents read and write its cells as before.
fn board_dialog(app: &AppHandle) -> tauri_plugin_dialog::FileDialogBuilder<tauri::Wry> {
    let dialog = app.dialog().file();
    if cfg!(target_os = "macos") {
        dialog.add_filter("recalc board", &["recalc"])
    } else {
        dialog
    }
}

/// The folder a board dialog named, as a package on macOS.
fn as_package(dir: std::path::PathBuf) -> std::path::PathBuf {
    if cfg!(target_os = "macos") && dir.extension().is_none_or(|e| e != "recalc") {
        let mut name = dir.as_os_str().to_owned();
        name.push(".recalc");
        return name.into();
    }
    dir
}

/// Make `next` the open board. A board New Board made and nobody touched
/// goes, as an untitled document closed unchanged does.
pub(crate) fn switch_to(
    app: &AppHandle,
    state: &Shared,
    ptys: &pty::Ptys,
    next: Board,
    dir: &std::path::Path,
) {
    let mut board = state.lock();
    detach_all(&mut board, ptys);
    let prev = std::mem::replace(&mut *board, next);
    board::push_recent(dir);
    board::set_title(app, dir);
    board::emit_full(app, &mut board);
    drop(board);
    let leftover = untouched(&prev).then(|| prev.engine.dir().to_path_buf());
    drop(prev);
    if let Some(old) = leftover.filter(|old| old != dir) {
        let _ = std::fs::remove_dir_all(old);
    }
}

/// File → Open…: a board package, an exported board's zip, or a template
/// (a picker when no path is given). Also what Finder hands the app for a
/// double-clicked `.recalc` or `.recalctemplate`. Returns the board's folder, or None
/// when a dialog was cancelled.
#[tauri::command]
pub(crate) async fn open_file(
    app: AppHandle,
    state: State<'_, Shared>,
    ptys: State<'_, pty::Ptys>,
    path: Option<String>,
) -> Result<Option<String>, String> {
    let path = match path {
        Some(p) => std::path::PathBuf::from(p),
        None => match picked(|| {
            app.dialog()
                .file()
                .set_title("Open a board")
                .add_filter("recalc board", &["recalc", "zip", "recalctemplate"])
                .blocking_pick_file()
        }) {
            Some(f) => f.into_path().map_err(|e| e.to_string())?,
            None => return Ok(None),
        },
    };
    open_path(&app, &state, &ptys, &path).map(|d| d.map(|d| d.display().to_string()))
}

/// Open what's at `path`: a folder as it is; an exported board unzipped
/// beside its zip, as Finder would; a template into a new board in the
/// boards folder, named for the template.
pub(crate) fn open_path(
    app: &AppHandle,
    state: &Shared,
    ptys: &pty::Ptys,
    path: &std::path::Path,
) -> Result<Option<std::path::PathBuf>, String> {
    if path.is_dir() {
        let next = Board::open(path)?;
        switch_to(app, state, ptys, next, path);
        return Ok(Some(path.to_path_buf()));
    }
    if recalc::bundle::is_zip(path) {
        // Opening it points cells at the documents that came with it.
        let into = recalc::bundle::unzip(path).map_err(|e| format!("{e:#}"))?;
        let next = Board::open(&into)?;
        switch_to(app, state, ptys, next, &into);
        return Ok(Some(into));
    }
    if !super::is_template(path) {
        return Err(format!(
            "{} isn't a board: open a .recalc board, an exported board's .zip, or a .recalctemplate",
            path.display()
        ));
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("{} isn't a template recalc can read ({e})", path.display()))?;
    let stem = path
        .file_stem()
        .map_or("board".into(), |s| s.to_string_lossy().into_owned());
    let into = fresh_folder(&boards_home(), &stem)?;
    let mut next = Board::open(&into)?;
    next.engine
        .apply_template(&text, Actor::Human)
        .map_err(|e| e.to_string())?;
    switch_to(app, state, ptys, next, &into);
    Ok(Some(into))
}

/// File → Export Board…: a copy of the board, and documents its cells name
/// from elsewhere, as one zip to send. Returns what it wrote, or None
/// when the dialog was cancelled.
#[tauri::command]
pub(crate) async fn export_board(
    app: AppHandle,
    state: State<'_, Shared>,
) -> Result<Option<String>, String> {
    let dir = state.lock().engine.dir().to_path_buf();
    let Some(file) = picked(|| {
        app.dialog()
            .file()
            .set_title("Export a copy of this board")
            .add_filter("zip", &["zip"])
            .set_file_name(recalc::bundle::zip_name(&dir))
            .blocking_save_file()
    }) else {
        return Ok(None);
    };
    let out = recalc::bundle::as_zip(&file.into_path().map_err(|e| e.to_string())?);
    // Never write the export into the board it packs, or over the board itself.
    let real = |p: &std::path::Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let parent = out.parent().map(real).unwrap_or_default();
    if parent.starts_with(real(&dir)) || real(&out) == real(&dir) {
        return Err("choose a place outside the board's own folder".into());
    }
    let done = recalc::bundle::export(&dir, &out).map_err(|e| format!("{e:#}"))?;
    // Said every time: an export is a copy, like an emailed spreadsheet.
    let together = if dir.join(".git").exists() {
        "This board is in a git repository, so committing its .recalc folder shares the board itself."
    } else {
        "To work on one board together, keep its folder somewhere you both open, like a shared drive."
    };
    Ok(Some(format!(
        "Exported a copy: {}. Changes made to the copy won't come back here. {together}",
        recalc::bundle::summary(&done, &out)
    )))
}

/// File → Save a Copy…: copy the board to a name and place the person picks,
/// then work in the copy, as Save As does. Returns what was said, or None
/// when the dialog was cancelled.
#[tauri::command]
pub(crate) async fn save_copy(
    app: AppHandle,
    state: State<'_, Shared>,
    ptys: State<'_, pty::Ptys>,
) -> Result<Option<String>, String> {
    let dir = state.lock().engine.dir().to_path_buf();
    let Some(file) = picked(|| {
        board_dialog(&app)
            .set_title("Save a copy of this board")
            .set_file_name(copy_name(&dir))
            .set_directory(dir.parent().unwrap_or(&dir))
            .blocking_save_file()
    }) else {
        return Ok(None);
    };
    let into = as_package(file.into_path().map_err(|e| e.to_string())?);
    if has_entries(&into) {
        return Err(occupied(&into));
    }
    recalc::bundle::copy_board(&dir, &into).map_err(|e| format!("{e:#}"))?;
    let next = Board::open(&into)?;
    switch_to(&app, &state, &ptys, next, &into);
    let copy = into.file_name().map_or_else(
        || into.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    Ok(Some(format!(
        "You're working in the copy, “{}”. The original is unchanged, in {}.",
        copy.strip_suffix(".recalc").unwrap_or(&copy),
        dir.display()
    )))
}

/// The name Save a Copy offers: the open board's, then "copy". It starts
/// beside the board, so macOS doesn't open wherever it last saved.
pub(crate) fn copy_name(dir: &std::path::Path) -> String {
    let name = dir
        .file_name()
        .map_or("board".into(), |n| n.to_string_lossy().into_owned());
    format!("{} copy", name.strip_suffix(".recalc").unwrap_or(&name))
}

/// Why a folder picked for a new board wasn't used.
pub(crate) fn occupied(dir: &std::path::Path) -> String {
    let name = dir.file_name().map_or_else(
        || dir.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    format!("“{name}” already has files in it, so recalc didn't use it. Choose a new name, or use File → Open… to open it.")
}

/// Whether a folder exists and holds anything at all.
pub(crate) fn has_entries(dir: &std::path::Path) -> bool {
    dir.read_dir()
        .is_ok_and(|mut entries| entries.next().is_some())
}

/// Open a board in another directory (a picker when no path is given),
/// detaching the old board's live sessions first.
#[tauri::command]
pub(crate) async fn open_board(
    app: AppHandle,
    state: State<'_, Shared>,
    ptys: State<'_, pty::Ptys>,
    path: Option<String>,
) -> Result<(), String> {
    let dir = match path {
        Some(p) => std::path::PathBuf::from(p),
        None => match picked(|| {
            app.dialog()
                .file()
                .set_title("Open a folder's board")
                .blocking_pick_folder()
        }) {
            Some(f) => f.into_path().map_err(|e| e.to_string())?,
            None => return Ok(()),
        },
    };
    let next = Board::open(&dir)?;
    switch_to(&app, &state, &ptys, next, &dir);
    Ok(())
}

/// Import a picked CSV file as literal cells with its top-left at `at`.
#[tauri::command]
pub(crate) async fn import_csv(
    app: AppHandle,
    state: State<'_, Shared>,
    at: String,
) -> Result<usize, String> {
    let Some(file) = picked(|| {
        app.dialog()
            .file()
            .add_filter("csv", &["csv"])
            .blocking_pick_file()
    }) else {
        return Ok(0);
    };
    let path = file.into_path().map_err(|e| e.to_string())?;
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut board = state.lock();
    let changed = board
        .engine
        .import_csv(&text, &at, Actor::Human)
        .map_err(|e| e.to_string())?;
    let n = changed.len();
    board.record_edits(changed);
    board::emit_changes(&app, &mut board);
    Ok(n)
}

/// Import a folder's files as rows, one path per cell down from `at`;
/// board-relative when the folder is inside the board.
#[tauri::command]
pub(crate) async fn import_files(
    app: AppHandle,
    state: State<'_, Shared>,
    at: String,
) -> Result<usize, String> {
    let Some(folder) = picked(|| app.dialog().file().blocking_pick_folder()) else {
        return Ok(0);
    };
    let folder = folder.into_path().map_err(|e| e.to_string())?;
    let mut board = state.lock();
    // Canonicalize both: macOS's picker may return /var for /private/var,
    // and the prefix strip would fail.
    let folder = folder.canonicalize().unwrap_or(folder);
    let root = board.engine.dir().to_path_buf();
    let root = root.canonicalize().unwrap_or(root);
    let mut paths: Vec<String> = std::fs::read_dir(&folder)
        .map_err(|e| e.to_string())?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') || !entry.file_type().ok()?.is_file() {
                return None;
            }
            let path = entry.path();
            let shown = path.strip_prefix(&root).unwrap_or(&path);
            Some(shown.display().to_string())
        })
        .collect();
    paths.sort();
    let text: String = paths.iter().map(|p| format!("{p}\n")).collect();
    let changed = board
        .engine
        .paste_text(&at, &text, Actor::Human)
        .map_err(|e| e.to_string())?;
    let n = changed.len();
    board.record_edits(changed);
    board::emit_changes(&app, &mut board);
    Ok(n)
}

/// One sheet as CSV (values only), through a save dialog: the one the window
/// shows, or the first.
#[tauri::command]
pub(crate) async fn export_csv(
    app: AppHandle,
    state: State<'_, Shared>,
    sheet: Option<String>,
) -> Result<(), String> {
    let sheet = state
        .lock()
        .engine
        .sheet_arg(sheet.as_deref())
        .map_err(|e| e.to_string())?;
    let Some(file) = picked(|| {
        app.dialog()
            .file()
            .set_file_name(format!("{}.csv", sheet.as_deref().unwrap_or("board")))
            .blocking_save_file()
    }) else {
        return Ok(());
    };
    let path = file.into_path().map_err(|e| e.to_string())?;
    let csv = grid_csv(&state.lock().engine, sheet.as_deref());
    std::fs::write(&path, csv).map_err(|e| e.to_string())
}

pub(crate) fn grid_csv(engine: &Engine, sheet: Option<&str>) -> String {
    use recalc::model::{a1_name, cell_coord, on_sheet};
    let a1_name = |c: u32, r: u32| on_sheet(sheet, &a1_name(c, r));
    let (mut cols, mut rows) = (0, 0);
    for cell in engine.all_cells() {
        if let Some((s, c, r)) = cell_coord(&cell.name) {
            if s != sheet {
                continue;
            }
            cols = cols.max(c);
            rows = rows.max(r);
        }
    }
    let mut out = String::new();
    for row in 1..=rows {
        let line: Vec<String> = (1..=cols)
            .map(|col| {
                csv_field(
                    engine
                        .cell(&a1_name(col, row))
                        .map(|c| c.value.as_str())
                        .unwrap_or(""),
                )
            })
            .collect();
        out.push_str(&line.join(","));
        out.push('\n');
    }
    out
}

/// One CSV field. A leading `=`, `+`, `-`, `@`, tab or CR is quoted with `'`
/// so spreadsheets don't run it as a formula; numbers keep their sign.
pub(crate) fn csv_field(v: &str) -> String {
    let injectable = matches!(v.chars().next(), Some('=' | '+' | '-' | '@' | '\t' | '\r'))
        && v.trim().parse::<f64>().is_err();
    let v = if injectable {
        format!("'{v}")
    } else {
        v.to_string()
    };
    if v.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", v.replace('"', "\"\""))
    } else {
        v
    }
}

/// The A1 grid as .xlsx, through a save dialog.
#[tauri::command]
pub(crate) async fn export_xlsx(app: AppHandle, state: State<'_, Shared>) -> Result<(), String> {
    let Some(file) = picked(|| {
        app.dialog()
            .file()
            .set_file_name("board.xlsx")
            .blocking_save_file()
    }) else {
        return Ok(());
    };
    let path = file.into_path().map_err(|e| e.to_string())?;
    let board = state.lock();
    let layout = board.engine.layout().unwrap_or_default();
    export::write(&board.engine, &layout, &path)
}

/// A cited document's text, for the source pane.
#[tauri::command]
pub(crate) async fn read_source(
    state: State<'_, Shared>,
    file: String,
) -> Result<source::Source, String> {
    let board = state.lock();
    if let Some(name) = file.strip_prefix("cell:") {
        let text = board
            .engine
            .cell(name)
            .map(|c| c.value.clone())
            .unwrap_or_default();
        return Ok(source::Source {
            file,
            kind: "cell",
            text,
        });
    }
    if file.starts_with("https://") || file.starts_with("http://") {
        let text = format!(
            "{file}\n\nA web page: recalc doesn't fetch it, so the passage wasn't checked."
        );
        return Ok(source::Source {
            file,
            kind: "url",
            text,
        });
    }
    let root = board.engine.dir().to_path_buf();
    drop(board);
    source::read(&root, &file)
}

#[tauri::command]
pub(crate) fn column_library(app: AppHandle) -> Vec<library::Column> {
    library::all(&app)
}

#[tauri::command]
pub(crate) fn save_column(app: AppHandle, column: library::Column) -> Result<(), String> {
    library::save(&app, column)
}

#[tauri::command]
pub(crate) fn forget_column(app: AppHandle, name: String) -> Result<(), String> {
    library::forget(&app, &name)
}

/// What a column holds, for the prompts that ask about it: documents (each
/// value names files, comma-separated, that exist) or anything else.
fn holds_documents(dir: &std::path::Path, values: &[String]) -> bool {
    let is_doc = |v: &str| {
        v.split(',')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .all(|p| dir.join(p).is_file())
    };
    let docs = values.iter().filter(|v| is_doc(v)).count();
    docs > 0 && docs * 2 >= values.len()
}

/// Add a question column: the question as header, and an agent prompt in
/// each row that has a value in the column it asks about (`about`, column A
/// unless given). A column of documents is read as documents; any other
/// column is the row's subject. Returns the header and prompt cells.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // a Tauri command takes its arguments flat
pub(crate) async fn add_question_column(
    app: AppHandle,
    state: State<'_, Shared>,
    column: library::Column,
    agent: String,
    col: Option<u32>,
    about: Option<u32>,
    sheet: Option<String>,
    model: Option<String>,
) -> Result<Value, String> {
    use recalc::model::{a1_name, cell_coord, on_sheet};
    column.validate()?;
    let model = model
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty());
    if let Some(m) = &model {
        if !m
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '/'))
        {
            return Err(format!("{m:?} is not a model name"));
        }
    }
    if agent.is_empty()
        || !agent
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(format!("{agent:?} is not an agent name"));
    }
    let mut board = state.lock();
    if model.is_some() && !board.engine.agent_takes_model(&agent) {
        return Err(format!(
            "{agent} can't take a model: its template under [agents] has no {{model}}; add {{model}} where the model flag goes"
        ));
    }
    let with_model = model
        .as_deref()
        .map(|m| format!(", model='{m}'"))
        .unwrap_or_default();
    // On the sheet the window shows; its prompts' unqualified refs mean it.
    let sheet = board
        .engine
        .sheet_arg(sheet.as_deref())
        .map_err(|e| e.to_string())?;
    let a1_name = |c: u32, r: u32| on_sheet(sheet.as_deref(), &a1_name(c, r));
    let (mut max_col, mut max_row) = (0, 0);
    for c in board.engine.all_cells() {
        if let Some((s, col, row)) = cell_coord(&c.name) {
            if s != sheet.as_deref() {
                continue;
            }
            max_col = max_col.max(col);
            max_row = max_row.max(row);
        }
    }
    let about = about.unwrap_or(1).max(1);
    let col = col.unwrap_or((max_col + 1).max(about + 1).max(2));
    if col == about {
        return Err("a question can't be about its own column".into());
    }
    let header = a1_name(col, 1);
    if board
        .engine
        .cell(&header)
        .is_some_and(|c| !c.value.is_empty() || c.source.is_some())
    {
        return Err(format!(
            "{header} already has a header; pick an empty column"
        ));
    }
    let letters: String = recalc::model::split_sheet(&header)
        .1
        .trim_end_matches(|c: char| c.is_ascii_digit())
        .into();
    let sentence = column.answer_sentence().replace('"', "'");
    let about_letters: String = recalc::model::a1_name(about, 1)
        .trim_end_matches(|c: char| c.is_ascii_digit())
        .into();
    let rows: Vec<(u32, String)> = (2..=max_row)
        .filter_map(|row| {
            let v = board
                .engine
                .cell(&a1_name(about, row))?
                .value
                .trim()
                .to_string();
            (!v.is_empty()).then_some((row, v))
        })
        .collect();
    let values: Vec<String> = rows.iter().map(|(_, v)| v.clone()).collect();
    let documents = holds_documents(board.engine.dir(), &values);
    let mut entries = vec![(header.clone(), column.question.trim().to_string())];
    if documents && board.engine.cell("instructions").is_none() {
        entries.push((
            "instructions".into(),
            recalc::agent::REVIEW_INSTRUCTIONS.trim().to_string(),
        ));
    }
    let mut answers = Vec::new();
    for (row, _) in &rows {
        let name = a1_name(col, *row);
        // Documents are read under the board's instructions; anything else is
        // the row's subject, answered in the column's form.
        let prompt = if documents {
            format!(
                "@{agent}(\"{{instructions}} Question: {{{letters}$1}} {sentence} Document: {{${about_letters}{row}}}\"{with_model})"
            )
        } else {
            format!(
                "@{agent}(\"Question: {{{letters}$1}} {sentence} Answer from what you know, in exactly that form, with no preamble; if you don't know, answer Not found. About: {{${about_letters}{row}}}\"{with_model})"
            )
        };
        entries.push((name.clone(), prompt));
        answers.push(name);
    }
    let previous: Vec<(String, String)> = entries
        .iter()
        .map(|(n, _)| (n.clone(), board.content_of(n)))
        .collect();
    board
        .engine
        .set_many(entries, Actor::Human)
        .map_err(|e| e.to_string())?;
    // The header takes A1's format; the answers take the answer type's.
    let header_format = board
        .engine
        .cell(&a1_name(1, 1))
        .and_then(|c| c.format.clone());
    if header_format.is_some() {
        board
            .engine
            .set_format(&header, header_format)
            .map_err(|e| e.to_string())?;
    }
    if let Some(f) = column.format() {
        for name in &answers {
            board
                .engine
                .set_format(name, Some(f.clone()))
                .map_err(|e| e.to_string())?;
        }
    }
    // Typed columns are strict, so a misfit is an error rather than a value
    // formulas trip on.
    if column.kind != recalc::model::AnswerKind::Text {
        let answer_type = column.answer_type();
        for name in &answers {
            board
                .engine
                .set_answer_type(name, Some(answer_type.clone()))
                .map_err(|e| e.to_string())?;
        }
    }
    board.record_edits(previous);
    board::emit_changes(&app, &mut board);
    Ok(json!({"header": header, "answers": answers, "documents": documents}))
}

/// Save the board as template text, through a save dialog.
#[tauri::command]
pub(crate) async fn save_board_as(app: AppHandle, state: State<'_, Shared>) -> Result<(), String> {
    let Some(file) = picked(|| {
        app.dialog()
            .file()
            .set_title("Save this board as a template")
            .add_filter("recalc template", &["recalctemplate"])
            .set_file_name("board.recalctemplate")
            .blocking_save_file()
    }) else {
        return Ok(());
    };
    let path = file.into_path().map_err(|e| e.to_string())?;
    let text = state.lock().engine.save_template(None);
    std::fs::write(&path, text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::holds_documents;

    #[test]
    fn a_column_holds_documents_when_its_values_name_files_that_exist() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("contracts")).unwrap();
        for f in ["a.md", "b.md"] {
            std::fs::write(dir.path().join("contracts").join(f), "x").unwrap();
        }
        let v = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(holds_documents(
            dir.path(),
            &v(&["contracts/a.md", "contracts/b.md"])
        ));
        // A row grouping an agreement with its amendment.
        assert!(holds_documents(
            dir.path(),
            &v(&["contracts/a.md, contracts/b.md"])
        ));
        // Names, IDs, and paths that don't exist are subjects, not documents.
        assert!(!holds_documents(dir.path(), &v(&["Acme", "Globex"])));
        assert!(!holds_documents(dir.path(), &v(&["contracts/missing.md"])));
        assert!(!holds_documents(dir.path(), &[]));
    }
}
