//! Commands the webview invokes. Writes are by `Actor::Human` and end by
//! pushing the changed cells; cell logic belongs to the engine.

use crate::board::{self, Board, Shared, Undo, TEMPLATES};
use crate::{export, library, pty};
use recalc::engine::{Axis, Choice, Engine, Fill};
use recalc::model::{Actor, AnswerType, CellKind, Format, Mark, MarkKind, Provenance};
use recalc::session;
use recalc::source;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;

mod boards;
mod onboard;
mod sessions;

#[cfg(target_os = "macos")]
pub(crate) use boards::open_path;
pub(crate) use boards::{boards_home, fresh_folder};
pub(crate) use sessions::start_session;
use sessions::{detach_all, picked};

pub fn handler() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        get_board,
        set_cell,
        edit_computed,
        delete_cells,
        fill,
        paste_text,
        paste_cells,
        paste_values,
        trigger,
        run_selection,
        stop,
        stop_all,
        restructure,
        sort_rows,
        set_format,
        set_answer_type,
        demos,
        open_demo,
        get_layout,
        set_layout,
        copy_text,
        resolve_decision,
        console,
        needs_confirm,
        set_series,
        set_mark,
        dismiss_suggestion,
        sessions::set_driver,
        set_budget,
        sessions::notify,
        #[cfg(debug_assertions)]
        crate::bridge::bridge_reply,
        templates,
        load_template,
        sessions::session_open,
        sessions::session_end,
        sessions::session_notify,
        sessions::driver_answer,
        open_script,
        sessions::session_handoff,
        sessions::driver_open,
        sessions::driver_end,
        sessions::driver_status,
        sessions::driver_handoff,
        undo,
        redo,
        sheet_edit,
        boards::new_board,
        boards::rename_board,
        onboard::agent_status,
        onboard::cli_status,
        onboard::install_cli,
        onboard::open_url,
        crate::update::update_status,
        crate::update::check_update,
        crate::update::set_auto_update,
        crate::update::install_update,
        boards::move_board,
        boards::open_file,
        boards::export_board,
        boards::save_copy,
        boards::open_board,
        boards::recent_boards,
        boards::import_csv,
        boards::import_files,
        boards::export_csv,
        boards::export_xlsx,
        boards::save_board_as,
        boards::read_source,
        boards::column_library,
        boards::save_column,
        boards::forget_column,
        boards::add_question_column,
        menu_enabled,
        agent_paths,
        model_aliases,
        connect_command,
        launch_notices,
        pty::pty_open,
        pty::pty_write,
        pty::pty_resize,
        pty::pty_close,
        pty::session_buffer,
    ]
}

#[tauri::command]
async fn get_board(state: State<'_, Shared>, ptys: State<'_, pty::Ptys>) -> Result<Value, String> {
    let mut board = state.lock();
    board.engine.take_changed();
    Ok(board::full_snapshot(&board, &ptys))
}

/// A person's write. In a cell with an answer type, a value that fits is stored in
/// its canonical form ("yes" → "Yes"); one that doesn't is kept as typed, and the
/// reply says what the cell takes, for the window to show.
#[tauri::command]
async fn set_cell(
    app: AppHandle,
    state: State<'_, Shared>,
    name: String,
    content: String,
) -> Result<Option<String>, String> {
    let mut board = state.lock();
    let mut warning = None;
    let typed = board.engine.cell(&name).and_then(|c| c.answer_type.clone());
    let content = match typed {
        Some(t)
            if CellKind::of_source(&content) == CellKind::Literal && !content.trim().is_empty() =>
        {
            match t.typed(&content) {
                Ok(canonical) => canonical,
                Err(wants) => {
                    warning = Some(format!("{name} takes {wants}; kept as you typed it"));
                    content
                }
            }
        }
        _ => content,
    };
    let prev = board.content_of(&name);
    let changed = prev != content;
    let literal = board.engine.cell(&name).is_some_and(|c| c.source.is_none());
    board
        .engine
        .set(&name, &content, Actor::Human)
        .map_err(|e| e.to_string())?;
    // An unchanged commit (Enter on a cell) must not eat the redo history.
    if changed {
        board.record([Undo::Edit {
            name,
            content: prev,
            literal,
        }]);
    }
    board::emit_changes(&app, &mut board);
    Ok(warning)
}

#[tauri::command]
async fn edit_computed(
    app: AppHandle,
    state: State<'_, Shared>,
    name: String,
    typed: String,
) -> Result<(), String> {
    let mut board = state.lock();
    board
        .engine
        .edit_computed(&name, typed)
        .map_err(|e| e.to_string())?;
    board::emit_changes(&app, &mut board);
    Ok(())
}

/// Delete a selection, keeping cells an agent defined (clearing those goes
/// through the single-cell edit decision). Returns the names kept.
#[tauri::command]
async fn delete_cells(
    app: AppHandle,
    state: State<'_, Shared>,
    names: Vec<String>,
) -> Result<Vec<String>, String> {
    let mut board = state.lock();
    let (kept, deletable): (Vec<String>, Vec<String>) = names.into_iter().partition(|n| {
        board
            .engine
            .cell(n)
            .is_some_and(|c| c.source.is_some() && matches!(c.def_by, Some(Actor::Agent(_))))
    });
    let removed = board
        .engine
        .delete_many(&deletable, Actor::Human)
        .map_err(|e| e.to_string())?;
    if !removed.is_empty() {
        board.record_edits(removed);
    }
    board::emit_changes(&app, &mut board);
    Ok(kept)
}

/// Pop `from`, apply it, and push its inverse onto `to`. Returns the cell to
/// select. A step that fails stays on `from`.
fn step(
    engine: &mut Engine,
    from: &mut Vec<Undo>,
    to: &mut Vec<Undo>,
) -> Result<Option<String>, String> {
    let Some(entry) = from.pop() else {
        return Ok(None);
    };
    match apply_undo(engine, &entry) {
        Ok((inverse, touched)) => {
            to.push(inverse);
            Ok(Some(touched))
        }
        Err(e) => {
            from.push(entry);
            Err(e)
        }
    }
}

/// Apply one undo step; returns its inverse and the cell to select.
/// Structural steps use the forward engine operations so references follow.
fn apply_undo(engine: &mut Engine, entry: &Undo) -> Result<(Undo, String), String> {
    use recalc::model::a1_name;
    let by = Actor::Human;
    let anchor = |sheet: &Option<String>, axis: Axis, at: u32| {
        let local = match axis {
            Axis::Rows => a1_name(1, at),
            Axis::Cols => a1_name(at, 1),
        };
        recalc::model::on_sheet(sheet.as_deref(), &local)
    };
    match entry {
        Undo::Group(steps) => {
            // Reverse order; the inverses, also applied in reverse, redo in order.
            let mut inverses = Vec::with_capacity(steps.len());
            let mut touched = String::new();
            for step in steps.iter().rev() {
                let (inverse, t) = apply_undo(engine, step)?;
                inverses.push(inverse);
                touched = t;
            }
            Ok((Undo::Group(inverses), touched))
        }
        Undo::Edit {
            name,
            content,
            literal,
        } => {
            let inverse = Undo::Edit {
                name: name.clone(),
                content: engine.content(name).unwrap_or_default(),
                literal: engine.cell(name).is_some_and(|c| c.source.is_none()),
            };
            let done = if *literal {
                engine
                    .set_literals(vec![(name.clone(), content.clone())], by)
                    .map(|_| ())
            } else {
                engine.set(name, content, by)
            };
            done.map_err(|e| e.to_string())?;
            Ok((inverse, name.clone()))
        }
        Undo::Insert {
            sheet,
            axis,
            at,
            n,
            removed,
        } => {
            engine
                .undelete(sheet.as_deref(), *axis, *at, *n, removed, by)
                .map_err(|e| e.to_string())?;
            let inverse = Undo::Delete {
                sheet: sheet.clone(),
                axis: *axis,
                at: *at,
                n: *n,
            };
            Ok((inverse, anchor(sheet, *axis, *at)))
        }
        Undo::Delete { sheet, axis, at, n } => {
            let removed = engine
                .delete(sheet.as_deref(), *axis, *at, *n, by)
                .map_err(|e| e.to_string())?;
            let inverse = Undo::Insert {
                sheet: sheet.clone(),
                axis: *axis,
                at: *at,
                n: *n,
                removed,
            };
            Ok((inverse, anchor(sheet, *axis, *at)))
        }
        Undo::MoveRows { sheet, moves } => {
            engine
                .move_rows(sheet.as_deref(), moves, by)
                .map_err(|e| e.to_string())?;
            let top = moves.iter().map(|&(_, to)| to).min().unwrap_or(1);
            let inverse = Undo::MoveRows {
                sheet: sheet.clone(),
                moves: moves.iter().map(|&(f, t)| (t, f)).collect(),
            };
            Ok((inverse, anchor(sheet, Axis::Rows, top)))
        }
    }
}

/// ⌘Z / ⇧⌘Z. Structural steps are refused while live sessions run, and
/// redraw the whole board.
fn undo_or_redo(
    app: &AppHandle,
    state: &State<'_, Shared>,
    ptys: &State<'_, pty::Ptys>,
    redo: bool,
) -> Result<Option<String>, String> {
    let mut board = state.lock();
    let b = &mut *board;
    let (from, to) = if redo {
        (&mut b.redo, &mut b.undo)
    } else {
        (&mut b.undo, &mut b.redo)
    };
    let structural = from.last().is_some_and(Undo::is_structural);
    if structural && !ptys.live_statuses().is_empty() {
        return Err("end the live agent sessions before moving rows or columns".into());
    }
    let name = step(&mut b.engine, from, to)?;
    if structural {
        board::emit_full(app, &mut board);
    } else {
        board::emit_changes(app, &mut board);
    }
    Ok(name)
}

#[tauri::command]
async fn undo(
    app: AppHandle,
    state: State<'_, Shared>,
    ptys: State<'_, pty::Ptys>,
) -> Result<Option<String>, String> {
    undo_or_redo(&app, &state, &ptys, false)
}

#[tauri::command]
async fn redo(
    app: AppHandle,
    state: State<'_, Shared>,
    ptys: State<'_, pty::Ptys>,
) -> Result<Option<String>, String> {
    undo_or_redo(&app, &state, &ptys, true)
}

/// ⌘D / ⌘R.
#[tauri::command]
async fn fill(
    app: AppHandle,
    state: State<'_, Shared>,
    from: String,
    to: String,
    dir: String,
) -> Result<(), String> {
    let dir = match dir.as_str() {
        "down" => Fill::Down,
        "right" => Fill::Right,
        _ => return Err(format!("bad fill direction {dir:?}")),
    };
    let mut board = state.lock();
    let changed = board
        .engine
        .fill(&from, &to, dir, Actor::Human)
        .map_err(|e| e.to_string())?;
    board.record_edits(changed);
    board::emit_changes(&app, &mut board);
    Ok(())
}

/// Paste text from outside the app: tab- or comma-separated literals.
#[tauri::command]
async fn paste_text(
    app: AppHandle,
    state: State<'_, Shared>,
    at: String,
    text: String,
) -> Result<usize, String> {
    let mut board = state.lock();
    let changed = board
        .engine
        .paste_text(&at, &text, Actor::Human)
        .map_err(|e| e.to_string())?;
    let n = changed.len();
    board.record_edits(changed);
    board::emit_changes(&app, &mut board);
    Ok(n)
}

/// ⇧⌘V: write each value as a literal, even one that parses as a source.
/// `None` and verified cells are skipped. Returns how many changed.
#[tauri::command]
async fn paste_values(
    app: AppHandle,
    state: State<'_, Shared>,
    at: String,
    block: Vec<Vec<Option<String>>>,
) -> Result<usize, String> {
    let mut board = state.lock();
    let changed = paste_values_on(&mut board, &at, &block)?;
    board::emit_changes(&app, &mut board);
    Ok(changed)
}

/// Records whether each cell held a literal, so undo restores text as text.
fn paste_values_on(
    board: &mut Board,
    at: &str,
    block: &[Vec<Option<String>>],
) -> Result<usize, String> {
    use recalc::model::{a1_coord, a1_name};
    let Some((c0, r0)) = a1_coord(at) else {
        return Err(format!("paste needs an A1 anchor, got {at:?}"));
    };
    let was_literal: std::collections::HashSet<String> = block
        .iter()
        .enumerate()
        .flat_map(|(r, row)| {
            row.iter()
                .enumerate()
                .filter(|(_, v)| v.is_some())
                .map(move |(c, _)| a1_name(c0 + c as u32, r0 + r as u32))
        })
        .filter(|n| board.engine.cell(n).is_some_and(|c| c.source.is_none()))
        .collect();
    let changed = board
        .engine
        .paste_values(at, block, Actor::Human)
        .map_err(|e| e.to_string())?;
    let n = changed.len();
    board.record(changed.into_iter().map(|(name, content)| Undo::Edit {
        literal: was_literal.contains(&name),
        name,
        content,
    }));
    Ok(n)
}

/// Paste cells copied inside the app: sources move with their references.
#[tauri::command]
async fn paste_cells(
    app: AppHandle,
    state: State<'_, Shared>,
    at: String,
    origin: String,
    block: Vec<Vec<Option<String>>>,
) -> Result<usize, String> {
    let mut board = state.lock();
    let changed = board
        .engine
        .paste_sources(&at, &origin, &block, Actor::Human)
        .map_err(|e| e.to_string())?;
    let n = changed.len();
    board.record_edits(changed);
    board::emit_changes(&app, &mut board);
    Ok(n)
}

#[tauri::command]
async fn trigger(app: AppHandle, state: State<'_, Shared>, name: String) -> Result<(), String> {
    let mut board = state.lock();
    board
        .engine
        .trigger(&name, Actor::Human)
        .map_err(|e| e.to_string())?;
    board::emit_changes(&app, &mut board);
    Ok(())
}

/// Set what agent runs on this board may spend (None: no limit).
#[tauri::command]
async fn set_budget(
    app: AppHandle,
    state: State<'_, Shared>,
    dollars: Option<f64>,
) -> Result<(), String> {
    let mut board = state.lock();
    board
        .engine
        .set_budget(dollars, Actor::Human)
        .map_err(|e| e.to_string())?;
    board::emit_status(&app, &mut board);
    Ok(())
}

/// Run every runnable cell in the selection; see `Engine::trigger_many`.
#[tauri::command]
async fn run_selection(
    app: AppHandle,
    state: State<'_, Shared>,
    names: Vec<String>,
    all: Option<bool>,
) -> Result<Value, String> {
    let mut board = state.lock();
    let out = board
        .engine
        .trigger_many(&names, Actor::Human, all.unwrap_or(false));
    board::emit_changes(&app, &mut board);
    Ok(json!({"ran": out.ran, "up_to_date": out.up_to_date, "need_confirm": out.need_confirm}))
}

/// Insert or delete rows/columns. Refused while a live session runs:
/// sessions are keyed by cell name and would lose their cell.
#[allow(clippy::too_many_arguments)] // a Tauri command takes its arguments flat
#[tauri::command]
async fn restructure(
    app: AppHandle,
    state: State<'_, Shared>,
    ptys: State<'_, pty::Ptys>,
    axis: String,
    at: u32,
    n: u32,
    insert: bool,
    sheet: Option<String>,
) -> Result<(), String> {
    let axis = match axis.as_str() {
        "rows" => Axis::Rows,
        "cols" => Axis::Cols,
        other => return Err(format!("bad axis {other:?}")),
    };
    let mut board = state.lock();
    if !ptys.live_statuses().is_empty() {
        return Err("end the live agent sessions before moving rows or columns".into());
    }
    let sheet = board
        .engine
        .sheet_arg(sheet.as_deref())
        .map_err(|e| e.to_string())?;
    let entry = if insert {
        board
            .engine
            .insert(sheet.as_deref(), axis, at, n, Actor::Human)
            .map_err(|e| e.to_string())?;
        Undo::Delete { sheet, axis, at, n }
    } else {
        let removed = board
            .engine
            .delete(sheet.as_deref(), axis, at, n, Actor::Human)
            .map_err(|e| e.to_string())?;
        Undo::Insert {
            sheet,
            axis,
            at,
            n,
            removed,
        }
    };
    board.record([entry]);
    board::emit_full(&app, &mut board);
    Ok(())
}

/// Sheets: add, rename, move, delete. Deleting can't be undone, as in Excel;
/// the window asks first.
#[tauri::command]
async fn sheet_edit(
    app: AppHandle,
    state: State<'_, Shared>,
    action: String,
    name: String,
    to: Option<String>,
    index: Option<usize>,
) -> Result<(), String> {
    let mut board = state.lock();
    let engine = &mut board.engine;
    match action.as_str() {
        "add" => engine.add_sheet(name.trim()),
        "rename" => engine.rename_sheet(&name, to.as_deref().unwrap_or_default().trim()),
        "duplicate" => engine.duplicate_sheet(
            &name,
            to.as_deref().unwrap_or_default().trim(),
            Actor::Human,
        ),
        "move" => engine.move_sheet(&name, index.unwrap_or(0)),
        "delete" => engine.delete_sheet(&name),
        other => return Err(format!("no sheet action {other:?}")),
    }
    .map_err(|e| e.to_string())?;
    board::emit_full(&app, &mut board);
    Ok(())
}

/// Sort data rows by a column. Refused while sessions run, like `restructure`.
#[allow(clippy::too_many_arguments)] // a Tauri command takes its arguments flat
#[tauri::command]
async fn sort_rows(
    app: AppHandle,
    state: State<'_, Shared>,
    ptys: State<'_, pty::Ptys>,
    col: u32,
    first: u32,
    last: u32,
    descending: bool,
    sheet: Option<String>,
) -> Result<(), String> {
    let mut board = state.lock();
    if !ptys.live_statuses().is_empty() {
        return Err("end the live agent sessions before sorting".into());
    }
    let sheet = board
        .engine
        .sheet_arg(sheet.as_deref())
        .map_err(|e| e.to_string())?;
    let moves = board
        .engine
        .sort_rows(sheet.as_deref(), col, first, last, descending, Actor::Human)
        .map_err(|e| e.to_string())?;
    if !moves.is_empty() {
        board.record([Undo::MoveRows {
            sheet,
            moves: moves.iter().map(|&(from, to)| (to, from)).collect(),
        }]);
    }
    board::emit_full(&app, &mut board);
    Ok(())
}

/// Replace the display format of each named cell (None clears it).
#[tauri::command]
async fn set_format(
    app: AppHandle,
    state: State<'_, Shared>,
    names: Vec<String>,
    format: Option<Format>,
) -> Result<(), String> {
    let mut board = state.lock();
    for name in &names {
        board
            .engine
            .set_format(name, format.clone())
            .map_err(|e| e.to_string())?;
    }
    board::emit_changes(&app, &mut board);
    Ok(())
}

/// Set or clear the cells' answer type. `kind` empty or "none" clears;
/// `strict` false notes a misfit instead of refusing it.
#[tauri::command]
async fn set_answer_type(
    app: AppHandle,
    state: State<'_, Shared>,
    names: Vec<String>,
    kind: Option<String>,
    options: Option<Vec<String>>,
    strict: Option<bool>,
) -> Result<(), String> {
    let answer_type = match kind.as_deref() {
        None | Some("") | Some("none") => None,
        Some(kind) => {
            let mut answer_type = AnswerType::new(kind.parse()?, &options.unwrap_or_default());
            answer_type.strict = strict.unwrap_or(true);
            answer_type.validate()?;
            Some(answer_type)
        }
    };
    let mut board = state.lock();
    for name in &names {
        board
            .engine
            .set_answer_type(name, answer_type.clone())
            .map_err(|e| e.to_string())?;
        // Apply the answer type's format unless the cell has its own.
        if let Some(f) = answer_type.as_ref().and_then(AnswerType::format) {
            if board.engine.cell(name).is_some_and(|c| c.format.is_none()) {
                board
                    .engine
                    .set_format(name, Some(f))
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    board::emit_changes(&app, &mut board);
    Ok(())
}

/// Stop a running or queued cell; its result is discarded.
#[tauri::command]
async fn stop(app: AppHandle, state: State<'_, Shared>, name: String) -> Result<bool, String> {
    let mut board = state.lock();
    let stopped = board
        .engine
        .stop(&name, Actor::Human)
        .map_err(|e| e.to_string())?;
    board::emit_changes(&app, &mut board);
    Ok(stopped)
}

#[tauri::command]
async fn stop_all(app: AppHandle, state: State<'_, Shared>) -> Result<usize, String> {
    let mut board = state.lock();
    let n = board
        .engine
        .stop_all(Actor::Human)
        .map_err(|e| e.to_string())?;
    board::emit_changes(&app, &mut board);
    Ok(n)
}

#[tauri::command]
async fn demos() -> Vec<Value> {
    recalc::demos::DEMOS
        .iter()
        .map(|d| json!({"name": d.name, "blurb": d.blurb}))
        .collect()
}

/// Open a demo: in place on an empty board, else in a new board in the
/// boards folder, named for the demo.
#[tauri::command]
async fn open_demo(
    app: AppHandle,
    state: State<'_, Shared>,
    ptys: State<'_, pty::Ptys>,
    name: String,
) -> Result<(), String> {
    let demo = recalc::demos::find(&name).ok_or_else(|| format!("no demo named {name:?}"))?;
    let empty_board_dir = {
        let board = state.lock();
        (board.engine.all_cells().count() == 0).then(|| board.engine.dir().to_path_buf())
    };
    let in_place = empty_board_dir.is_some();
    let dir = match empty_board_dir {
        Some(dir) => dir,
        None => boards::fresh_folder(&boards::boards_home(), demo.name)?,
    };
    let text = recalc::demos::unpack(demo, &dir).map_err(|e| e.to_string())?;
    if in_place {
        let mut board = state.lock();
        board
            .engine
            .apply_template(text, Actor::Human)
            .map_err(|e| e.to_string())?;
        board::emit_changes(&app, &mut board);
        return Ok(());
    }
    let mut next = Board::open(&dir)?;
    next.engine
        .apply_template(text, Actor::Human)
        .map_err(|e| e.to_string())?;
    boards::switch_to(&app, &state, &ptys, next, &dir);
    Ok(())
}

/// The board's saved view layout, as the frontend wrote it.
#[tauri::command]
async fn get_layout(state: State<'_, Shared>) -> Result<String, String> {
    state.lock().engine.layout().map_err(|e| e.to_string())
}

#[tauri::command]
async fn set_layout(state: State<'_, Shared>, json: String) -> Result<(), String> {
    state
        .lock()
        .engine
        .set_layout(&json)
        .map_err(|e| e.to_string())
}

/// Put text on the system clipboard (the grid's copy-as-Markdown).
#[tauri::command]
async fn copy_text(app: AppHandle, text: String) -> Result<(), String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    app.clipboard().write_text(text).map_err(|e| e.to_string())
}

#[tauri::command]
async fn resolve_decision(
    app: AppHandle,
    state: State<'_, Shared>,
    name: String,
    index: Option<usize>,
    text: Option<String>,
) -> Result<(), String> {
    let choice = match (index, text) {
        (Some(i), _) => Choice::Index(i),
        (None, Some(t)) => Choice::Text(t),
        (None, None) => return Err("resolve needs an option index or text".into()),
    };
    let mut board = state.lock();
    // Pin is a human edit and goes on the undo stack; answering an agent's
    // question is not, since the agent has already acted on it.
    let is_edit = board
        .engine
        .cell(&name)
        .and_then(|c| c.decision.as_ref())
        .is_some_and(|d| d.options.iter().any(|o| o.value == "pin"));
    let before = board.content_of(&name);
    board
        .engine
        .resolve_decision(&name, choice, Actor::Human)
        .map_err(|e| e.to_string())?;
    if is_edit && board.content_of(&name) != before {
        board.record_edits([(name.clone(), before)]);
    }
    board::emit_changes(&app, &mut board);
    Ok(())
}

/// A command cell's resolved command, cwd, and watch/every, for the inspector.
#[tauri::command]
async fn console(state: State<'_, Shared>, name: String) -> Result<Value, String> {
    let board = state.lock();
    match board.engine.console_info(&name) {
        Some(info) => Ok(json!({
            "command": info.command,
            "cwd": info.cwd.display().to_string(),
            "watch": info.watch,
            "every": info.every,
        })),
        None => Err(format!("{name} is not a command cell")),
    }
}

#[tauri::command]
async fn needs_confirm(state: State<'_, Shared>, name: String) -> Result<bool, String> {
    Ok(state.lock().engine.needs_confirm(&name))
}

/// Toggle inline sparkline tracking for a cell.
#[tauri::command]
async fn set_series(
    app: AppHandle,
    state: State<'_, Shared>,
    name: String,
    on: bool,
) -> Result<(), String> {
    let mut board = state.lock();
    board
        .engine
        .set_series(&name, on)
        .map_err(|e| e.to_string())?;
    board::emit_changes(&app, &mut board);
    Ok(())
}

/// Dismiss a cell's suggestion (the one with `value`, or all).
#[tauri::command]
async fn dismiss_suggestion(
    app: AppHandle,
    state: State<'_, Shared>,
    name: String,
    value: Option<String>,
) -> Result<(), String> {
    let mut board = state.lock();
    board
        .engine
        .dismiss_suggestions(&name, value.as_deref(), Actor::Human)
        .map_err(|e| e.to_string())?;
    board::emit_changes(&app, &mut board);
    Ok(())
}

/// Verify or flag cells (with an optional note), or clear their marks.
#[tauri::command]
async fn set_mark(
    app: AppHandle,
    state: State<'_, Shared>,
    names: Vec<String>,
    kind: Option<String>,
    note: Option<String>,
) -> Result<(), String> {
    let mark = match kind.as_deref() {
        None | Some("") => None,
        Some("verified") => Some(MarkKind::Verified),
        Some("flagged") => Some(MarkKind::Flagged),
        Some(other) => return Err(format!("unknown mark {other:?}")),
    }
    .map(|kind| Mark {
        kind,
        note: note.filter(|n| !n.trim().is_empty()),
        by: Actor::Human,
        at: Provenance::now(Actor::Human).at,
    });
    let mut board = state.lock();
    for name in &names {
        board
            .engine
            .set_mark(name, mark.clone())
            .map_err(|e| e.to_string())?;
    }
    board::emit_changes(&app, &mut board);
    Ok(())
}

#[tauri::command]
async fn templates(state: State<'_, Shared>) -> Result<Vec<Value>, String> {
    let mut out: Vec<Value> = TEMPLATES
        .iter()
        .map(|(name, text)| json!({ "name": name, "note": board::template_note(text) }))
        .collect();
    // Boards saved with `recalc save examples/<name>.recalctemplate`, named by
    // path; `.recalc` text files from before the rename too.
    let dir = state.lock().engine.dir().join("examples");
    let mut saved: Vec<_> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && is_template(p))
        .collect();
    saved.sort();
    for path in saved {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let name = format!(
            "examples/{}",
            path.file_name().unwrap_or_default().to_string_lossy()
        );
        out.push(json!({ "name": name, "note": board::template_note(&text) }));
    }
    Ok(out)
}

/// A board's cells saved as text: `.recalctemplate`, or `.recalc` from before.
pub(crate) fn is_template(path: &std::path::Path) -> bool {
    path.extension()
        .is_some_and(|x| x == "recalctemplate" || x == "recalc")
}

/// Apply a shipped template to the current board (adds/overwrites its cells).
#[tauri::command]
async fn load_template(
    app: AppHandle,
    state: State<'_, Shared>,
    name: String,
) -> Result<(), String> {
    let mut board = state.lock();
    let text = match TEMPLATES.iter().find(|(n, _)| *n == name) {
        Some((_, text)) => text.to_string(),
        None => {
            // A board's own saved template: a file directly in its examples/.
            let file = name
                .strip_prefix("examples/")
                .filter(|f| {
                    is_template(std::path::Path::new(f)) && !f.contains('/') && !f.contains("..")
                })
                .ok_or_else(|| format!("no such template: {name}"))?;
            std::fs::read_to_string(board.engine.dir().join("examples").join(file))
                .map_err(|e| format!("{name}: {e}"))?
        }
    };
    board
        .engine
        .apply_template(&text, Actor::Human)
        .map_err(|e| e.to_string())?;
    board::emit_changes(&app, &mut board);
    Ok(())
}

/// What the window should tell the person once at launch.
#[tauri::command]
fn launch_notices() -> Vec<String> {
    let mut out = Vec::new();
    if std::env::current_exe().is_ok_and(|p| board::from_disk_image(&p)) {
        out.push("recalc is running from its disk image. Drag it to Applications and open it from there: agent sessions connect to the board through recalc where it runs, so ejecting the image would cut them off.".to_string());
    }
    out
}

/// The command that connects a person's own Claude Code to the board, with
/// this binary's full path: an installed app puts `recalc` on no PATH.
#[tauri::command]
fn connect_command() -> String {
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "recalc".into());
    format!(
        "claude mcp add recalc -- {} mcp",
        recalc::interp::shell_quote(&exe)
    )
}

/// The board's `[models]` aliases, for picking a column's model.
#[tauri::command]
fn model_aliases(state: State<'_, Shared>) -> Vec<String> {
    let board = state.lock();
    let mut names: Vec<String> = board.engine.config().models.keys().cloned().collect();
    names.sort();
    names
}

/// Where each agent's executable resolves, for the Connect panel. Custom
/// `[agents]` resolve by their template's first word.
#[tauri::command]
fn agent_paths(state: State<'_, Shared>) -> Vec<Value> {
    let board = state.lock();
    let config = board.engine.config();
    let mut out = Vec::new();
    for name in ["claude", "codex", "pi"] {
        if config.agents.contains_key(name) {
            continue; // shadowed by a custom template, listed below
        }
        let wanted = config
            .paths
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.to_string());
        let found = recalc::shell_env::which(&wanted);
        out.push(json!({
            "name": name,
            "configured": config.paths.get(name),
            "found": found.map(|p| p.display().to_string()),
            // The builtin templates request JSON output, which carries usage.
            "reports_usage": true,
        }));
    }
    let mut custom: Vec<_> = config.agents.iter().collect();
    custom.sort();
    for (name, template) in custom {
        let word = template.split_whitespace().next().unwrap_or("");
        out.push(json!({
            "name": name,
            "configured": template,
            "found": recalc::shell_env::which(word).map(|p| p.display().to_string()),
            "reports_usage": template.contains("--output-format json")
                || template.contains("--json")
                || template.contains("--mode json"),
        }));
    }
    out
}

/// Gray out native menu items the current selection can't use.
#[tauri::command]
fn menu_enabled(menus: State<'_, crate::menu::Items>, items: Vec<(String, bool)>) {
    for (id, on) in items {
        menus.set_enabled(&id, on);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn csv_fields_are_quoted_and_formula_injection_is_neutralised() {
        use super::boards::csv_field;
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a, b"), "\"a, b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field("=1+1"), "'=1+1");
        assert_eq!(csv_field("@SUM(A1)"), "'@SUM(A1)");
        assert_eq!(csv_field("+cmd|' /C calc'!A0"), "'+cmd|' /C calc'!A0");
        assert_eq!(csv_field("-2"), "-2", "a negative number is a number");
        assert_eq!(csv_field("+3.5"), "+3.5");
        assert_eq!(csv_field("-2+3"), "'-2+3");
    }

    use super::boards::grid_csv;
    use super::sessions::one_line;
    use super::*;

    #[tokio::test]
    async fn csv_covers_the_a1_extent_and_escapes() {
        let dir = tempfile::tempdir().unwrap();
        let mut engine = Engine::open(dir.path()).unwrap();
        engine.set("A1", "plain", Actor::Human).unwrap();
        engine.set("B2", "with, comma", Actor::Human).unwrap();
        engine.set("C1", "quote \" here", Actor::Human).unwrap();
        engine.set("named", "skipped", Actor::Human).unwrap();
        assert_eq!(
            grid_csv(&engine, None),
            "plain,,\"quote \"\" here\"\n,\"with, comma\",\n"
        );
    }

    #[test]
    fn a_new_board_takes_the_next_free_name_and_never_an_existing_folder() {
        use super::boards::fresh_folder;
        let home = tempfile::tempdir().unwrap();
        let first = fresh_folder(home.path(), "Untitled board").unwrap();
        let second = fresh_folder(home.path(), "Untitled board").unwrap();
        let ext = if cfg!(target_os = "macos") {
            ".recalc"
        } else {
            ""
        };
        assert_eq!(first, home.path().join(format!("Untitled board{ext}")));
        assert_eq!(second, home.path().join(format!("Untitled board 2{ext}")));
        // A folder someone made by hand under the next name is passed over, not opened.
        std::fs::create_dir(home.path().join(format!("Untitled board 3{ext}"))).unwrap();
        std::fs::write(
            home.path().join(format!("Untitled board 3{ext}/notes.txt")),
            "x",
        )
        .unwrap();
        let fourth = fresh_folder(home.path(), "Untitled board").unwrap();
        assert_eq!(fourth, home.path().join(format!("Untitled board 4{ext}")));
        assert_eq!(std::fs::read_dir(&fourth).unwrap().count(), 0, "made empty");
        // A name with a dot in it is still a package, not a file named ".2".
        let dotted = fresh_folder(home.path(), "Plan v1.2").unwrap();
        assert!(dotted
            .to_string_lossy()
            .ends_with(&format!("Plan v1.2{ext}")));
    }

    #[test]
    fn save_a_copy_offers_the_open_boards_name() {
        use super::boards::copy_name;
        assert_eq!(
            copy_name(std::path::Path::new("/b/Q3 review.recalc")),
            "Q3 review copy"
        );
        assert_eq!(copy_name(std::path::Path::new("/repos/api")), "api copy");
    }

    fn edit(name: &str, content: &str) -> Undo {
        Undo::Edit {
            name: name.into(),
            content: content.into(),
            literal: false,
        }
    }

    fn src(engine: &Engine, name: &str) -> Option<String> {
        engine.content(name)
    }

    #[tokio::test]
    async fn undo_stack_restores_previous_content() {
        let dir = tempfile::tempdir().unwrap();
        let mut engine = Engine::open(dir.path()).unwrap();
        engine.set("A1", "1", Actor::Human).unwrap();
        let mut undo = vec![edit("A1", &engine.content("A1").unwrap())];
        let mut redo = Vec::new();
        engine.set("A1", "2", Actor::Human).unwrap();

        let touched = step(&mut engine, &mut undo, &mut redo).unwrap();
        assert_eq!(touched.as_deref(), Some("A1"));
        assert_eq!(engine.cell("A1").unwrap().value, "1");
        let touched = step(&mut engine, &mut redo, &mut undo).unwrap();
        assert_eq!(touched.as_deref(), Some("A1"));
        assert_eq!(engine.cell("A1").unwrap().value, "2");
        // Empty stack is a quiet no-op.
        assert_eq!(step(&mut engine, &mut Vec::new(), &mut undo).unwrap(), None);
    }

    #[tokio::test]
    async fn fill_results_are_valid_undo_entries() {
        let dir = tempfile::tempdir().unwrap();
        let mut engine = Engine::open(dir.path()).unwrap();
        engine.set("A1", "1", Actor::Human).unwrap();
        engine.set("A2", "2", Actor::Human).unwrap();
        engine.set("B1", "= {A1} * 2", Actor::Human).unwrap();
        let filled = engine.fill("B1", "B2", Fill::Down, Actor::Human).unwrap();
        let mut undo: Vec<Undo> = filled.iter().map(|(n, c)| edit(n, c)).collect();
        let mut redo = Vec::new();
        let touched = step(&mut engine, &mut undo, &mut redo).unwrap();
        assert_eq!(touched.as_deref(), Some("B2"));
        assert_eq!(engine.content("B2").as_deref(), Some(""));
    }

    #[test]
    fn delete_rows_undo_restores_cells_marks_formats_and_refs() {
        let dir = tempfile::tempdir().unwrap();
        let mut board = Board::open(dir.path()).unwrap();
        let e = &mut board.engine;
        for (n, c) in [("A1", "1"), ("A2", "2"), ("A3", "3"), ("A4", "4")] {
            e.set(n, c, Actor::Human).unwrap();
        }
        e.set("B2", "= {A2} * 10", Actor::Human).unwrap();
        e.set("C1", "= {A2} + {A4}", Actor::Human).unwrap();
        e.set("C2", "= sum({A1:A3})", Actor::Human).unwrap();
        let bold = Format {
            bold: true,
            ..Format::default()
        };
        e.set_format("A2", Some(bold.clone())).unwrap();
        let verified = Mark {
            kind: MarkKind::Verified,
            note: Some("checked".into()),
            by: Actor::Human,
            at: 1,
        };
        e.set_mark("A2", Some(verified.clone())).unwrap();

        let removed = board
            .engine
            .delete(None, Axis::Rows, 2, 2, Actor::Human)
            .unwrap();
        board.record([Undo::Insert {
            sheet: None,
            axis: Axis::Rows,
            at: 2,
            n: 2,
            removed,
        }]);
        let e = &board.engine;
        assert_eq!(src(e, "A2").as_deref(), Some("4"), "A4 closed the gap");
        assert!(e.cell("A3").is_none());
        // The ref into a deleted row is #REF!, not whatever moved up into it.
        assert_eq!(src(e, "C1").as_deref(), Some("= {#REF!} + {A2}"));

        let b = &mut board;
        let touched = step(&mut b.engine, &mut b.undo, &mut b.redo).unwrap();
        assert_eq!(touched.as_deref(), Some("A2"));
        let e = &board.engine;
        for (n, c) in [
            ("A1", "1"),
            ("A2", "2"),
            ("A3", "3"),
            ("A4", "4"),
            ("B2", "= {A2} * 10"),
            ("C1", "= {A2} + {A4}"),
            ("C2", "= sum({A1:A3})"),
        ] {
            assert_eq!(src(e, n).as_deref(), Some(c), "{n}");
        }
        let a2 = e.cell("A2").unwrap();
        assert_eq!(a2.format.as_ref(), Some(&bold));
        assert_eq!(a2.mark.as_ref(), Some(&verified));
        assert_eq!(e.cell("B2").unwrap().value, "20");
        assert!(board.undo.is_empty());

        let b = &mut board;
        step(&mut b.engine, &mut b.redo, &mut b.undo).unwrap();
        assert_eq!(src(&board.engine, "A2").as_deref(), Some("4"));
        assert!(board.engine.cell("B2").is_none());
        let b = &mut board;
        step(&mut b.engine, &mut b.undo, &mut b.redo).unwrap();
        assert_eq!(src(&board.engine, "B2").as_deref(), Some("= {A2} * 10"));
        assert_eq!(
            board.engine.cell("A2").unwrap().mark.as_ref(),
            Some(&verified)
        );
    }

    #[test]
    fn insert_cols_undo_and_redo_move_refs_back_and_forth() {
        let dir = tempfile::tempdir().unwrap();
        let mut board = Board::open(dir.path()).unwrap();
        let e = &mut board.engine;
        e.set("A1", "5", Actor::Human).unwrap();
        e.set("B1", "7", Actor::Human).unwrap();
        e.set("C1", "= {A1} + {B1}", Actor::Human).unwrap();
        e.insert(None, Axis::Cols, 2, 1, Actor::Human).unwrap();
        board.record([Undo::Delete {
            sheet: None,
            axis: Axis::Cols,
            at: 2,
            n: 1,
        }]);
        assert_eq!(src(&board.engine, "D1").as_deref(), Some("= {A1} + {C1}"));

        let b = &mut board;
        assert_eq!(
            step(&mut b.engine, &mut b.undo, &mut b.redo)
                .unwrap()
                .as_deref(),
            Some("B1")
        );
        assert_eq!(src(&board.engine, "C1").as_deref(), Some("= {A1} + {B1}"));
        assert!(board.engine.cell("D1").is_none());
        let b = &mut board;
        step(&mut b.engine, &mut b.redo, &mut b.undo).unwrap();
        assert_eq!(src(&board.engine, "D1").as_deref(), Some("= {A1} + {C1}"));
        assert_eq!(board.engine.cell("D1").unwrap().value, "12");
    }

    #[test]
    fn sort_undo_restores_order_and_refs() {
        let dir = tempfile::tempdir().unwrap();
        let mut board = Board::open(dir.path()).unwrap();
        let e = &mut board.engine;
        for (n, c) in [("A1", "name"), ("A2", "c"), ("A3", "a"), ("A4", "b")] {
            e.set(n, c, Actor::Human).unwrap();
        }
        e.set("B2", "3", Actor::Human).unwrap();
        e.set("B3", "1", Actor::Human).unwrap();
        e.set("B4", "2", Actor::Human).unwrap();
        e.set("D1", "= {B2} * 100", Actor::Human).unwrap();
        let moves = e.sort_rows(None, 1, 2, 4, false, Actor::Human).unwrap();
        board.record([Undo::MoveRows {
            sheet: None,
            moves: moves.iter().map(|&(f, t)| (t, f)).collect(),
        }]);
        let col = |e: &Engine, c: &str| -> Vec<String> {
            (2..=4)
                .map(|r| src(e, &format!("{c}{r}")).unwrap_or_default())
                .collect()
        };
        assert_eq!(col(&board.engine, "A"), ["a", "b", "c"]);
        assert_eq!(src(&board.engine, "D1").as_deref(), Some("= {B4} * 100"));

        let b = &mut board;
        step(&mut b.engine, &mut b.undo, &mut b.redo).unwrap();
        assert_eq!(col(&board.engine, "A"), ["c", "a", "b"]);
        assert_eq!(col(&board.engine, "B"), ["3", "1", "2"]);
        assert_eq!(src(&board.engine, "D1").as_deref(), Some("= {B2} * 100"));
        let b = &mut board;
        step(&mut b.engine, &mut b.redo, &mut b.undo).unwrap();
        assert_eq!(col(&board.engine, "A"), ["a", "b", "c"]);
        assert_eq!(src(&board.engine, "D1").as_deref(), Some("= {B4} * 100"));
    }

    #[test]
    fn an_edit_after_undo_clears_redo_and_the_stack_stays_capped() {
        let dir = tempfile::tempdir().unwrap();
        let mut board = Board::open(dir.path()).unwrap();
        board
            .engine
            .insert(None, Axis::Rows, 1, 1, Actor::Human)
            .unwrap();
        board.record([Undo::Delete {
            sheet: None,
            axis: Axis::Rows,
            at: 1,
            n: 1,
        }]);
        let b = &mut board;
        step(&mut b.engine, &mut b.undo, &mut b.redo).unwrap();
        assert_eq!(board.redo.len(), 1);
        board.engine.set("A1", "x", Actor::Human).unwrap();
        board.record_edits([("A1".to_string(), String::new())]);
        assert!(board.redo.is_empty());
        for i in 0..150 {
            board.record_edits([(format!("A{}", i + 1), String::new())]);
        }
        assert_eq!(board.undo.len(), 100);
    }

    #[test]
    fn one_paste_is_one_undo() {
        let dir = tempfile::tempdir().unwrap();
        let mut board = Board::open(dir.path()).unwrap();
        let block = vec![vec![
            Some("1".to_string()),
            Some("2".into()),
            Some("3".into()),
        ]];
        let changed = board
            .engine
            .paste_values("A1", &block, Actor::Human)
            .unwrap();
        board.record(changed.into_iter().map(|(name, content)| Undo::Edit {
            name,
            content,
            literal: true,
        }));
        assert_eq!(board.undo.len(), 1, "the paste is one step");
        let b = &mut board;
        step(&mut b.engine, &mut b.undo, &mut b.redo).unwrap();
        for n in ["A1", "B1", "C1"] {
            assert!(
                board.engine.cell(n).is_none_or(|c| c.value.is_empty()),
                "{n} undone"
            );
        }
        let b = &mut board;
        step(&mut b.engine, &mut b.redo, &mut b.undo).unwrap();
        assert_eq!(
            board.engine.cell("C1").unwrap().value,
            "3",
            "redo puts it all back"
        );
    }

    #[test]
    fn paste_values_lands_text_that_reads_like_a_source() {
        let dir = tempfile::tempdir().unwrap();
        let mut board = Board::open(dir.path()).unwrap();
        board.engine.set("A2", "old", Actor::Human).unwrap();
        let block = vec![
            vec![Some("= 1+1".to_string()), None, Some("@claude('x')".into())],
            vec![Some("$ echo hi".to_string()), Some("! rm -rf /".into())],
        ];
        assert_eq!(paste_values_on(&mut board, "A1", &block).unwrap(), 4);
        let e = &board.engine;
        for (n, v) in [
            ("A1", "= 1+1"),
            ("C1", "@claude('x')"),
            ("A2", "$ echo hi"),
            ("B2", "! rm -rf /"),
        ] {
            let c = e.cell(n).unwrap();
            assert!(c.source.is_none(), "{n} is a literal");
            assert_eq!(c.value, v);
        }
        assert!(e.cell("B1").is_none(), "None skips the cell");

        // One undo takes the paste back, and redo keeps it text.
        let b = &mut board;
        step(&mut b.engine, &mut b.undo, &mut b.redo).unwrap();
        assert_eq!(board.engine.content("A1").as_deref(), Some(""));
        assert_eq!(board.engine.content("A2").as_deref(), Some("old"));
        let b = &mut board;
        step(&mut b.engine, &mut b.redo, &mut b.undo).unwrap();
        let a1 = board.engine.cell("A1").unwrap();
        assert!(a1.source.is_none());
        assert_eq!(a1.value, "= 1+1");
        assert!(board.engine.cell("A2").unwrap().source.is_none());
    }

    #[test]
    fn notify_messages_stay_on_one_line() {
        assert_eq!(one_line("a\nb"), "a");
        assert_eq!(one_line(&"x".repeat(100)).chars().count(), 80);
    }
}

/// `path` as a file inside the board at `dir`, after symlinks and `..`; else
/// an error. A cell's source names it, so it's taken as untrusted.
fn board_file(dir: &std::path::Path, path: &str) -> Result<std::path::PathBuf, String> {
    let root = dir.canonicalize().map_err(|e| e.to_string())?;
    let file = root
        .join(path)
        .canonicalize()
        .map_err(|_| format!("{path} isn't a file in this board"))?;
    if !file.starts_with(&root) || !file.is_file() {
        return Err(format!("{path} isn't a file in this board"));
    }
    Ok(file)
}

/// Open a script a cell runs (`! python3 scripts/clean.py`) in the person's
/// default text editor, to read or change it. Only files inside the board: the
/// path comes from a cell's source. A text editor, so a script is shown, never
/// run.
#[tauri::command]
async fn open_script(state: State<'_, Shared>, path: String) -> Result<(), String> {
    let dir = state.lock().engine.dir().to_path_buf();
    let file = board_file(&dir, &path)?;
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.arg("-t");
        c
    };
    #[cfg(not(target_os = "macos"))]
    let mut cmd = std::process::Command::new("xdg-open");
    cmd.arg(&file)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("couldn't open {path}: {e}"))
}

#[cfg(test)]
mod script_tests {
    use super::board_file;

    #[test]
    #[cfg(unix)]
    fn a_script_opens_only_from_inside_the_board() {
        let outside = tempfile::tempdir().unwrap();
        let board = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(board.path().join("scripts")).unwrap();
        std::fs::write(board.path().join("scripts/clean.py"), "print(1)").unwrap();
        std::fs::write(outside.path().join("secret.txt"), "x").unwrap();
        assert!(board_file(board.path(), "scripts/clean.py").is_ok());
        assert!(board_file(board.path(), "./scripts/clean.py").is_ok());
        let out = outside.path().join("secret.txt");
        assert!(
            board_file(board.path(), out.to_str().unwrap()).is_err(),
            "an absolute path elsewhere"
        );
        let up = format!(
            "../{}/secret.txt",
            outside.path().file_name().unwrap().to_str().unwrap()
        );
        assert!(board_file(board.path(), &up).is_err(), "climbing out");
        std::os::unix::fs::symlink(&out, board.path().join("scripts/link.py")).unwrap();
        assert!(
            board_file(board.path(), "scripts/link.py").is_err(),
            "a link that leads out"
        );
        assert!(
            board_file(board.path(), "scripts").is_err(),
            "a folder isn't a script"
        );
        assert!(board_file(board.path(), "scripts/missing.py").is_err());
    }
}
