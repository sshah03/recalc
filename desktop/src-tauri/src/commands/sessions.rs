//! Live agent sessions and the driver.

use super::*;

// ---- live agent sessions ----

/// Start a live agent session for an `@` cell; a no-op while one runs.
#[tauri::command]
pub(crate) async fn session_open(
    app: AppHandle,
    state: State<'_, Shared>,
    ptys: State<'_, pty::Ptys>,
    name: String,
    cols: u16,
    rows: u16,
    fresh: Option<bool>,
) -> Result<Option<String>, String> {
    let mut board = state.lock();
    start_session(
        &app,
        &mut board,
        &ptys,
        &name,
        pty::term_size(cols, rows),
        fresh.unwrap_or(false),
    )
}

/// A new conversation in place of one this computer doesn't have.
pub(crate) const NOT_ON_THIS_COMPUTER: &str =
    "Its last conversation isn't on this computer, so this is a new one. The board's cells, notes and marks carry over.";

/// Start (or reattach to) a cell's live session. Runs under the Board lock so
/// two rapid opens don't replace each other's session. Returns a note for the
/// person when the session isn't the one they'd expect.
pub(crate) fn start_session(
    app: &AppHandle,
    board: &mut Board,
    ptys: &pty::Ptys,
    name: &str,
    size: portable_pty::PtySize,
    fresh: bool,
) -> Result<Option<String>, String> {
    if ptys.has_live(name) {
        return Ok(None);
    }
    let tmux = session::available().then(|| session::name(board.engine.dir(), name));
    // Reuse the cell's conversation id while its tmux session lives; else
    // record a fresh one so any terminal can resume the conversation.
    let alive = tmux.as_deref().is_some_and(session::alive);
    let existing = board.engine.cell(name).and_then(|c| c.session_id.clone());
    // Its terminal is gone (a restart) but its conversation isn't: continue
    // it, unless the person asked for a new one.
    let agent = board.engine.agent_of(name).unwrap_or_default();
    let codex = agent == "codex";
    let wanted = existing.as_deref().filter(|_| !alive && !fresh);
    // A board copied from another computer names conversations this one never had.
    let program = board.engine.program_of(name).unwrap_or_default();
    let elsewhere = wanted.is_some_and(|id| !session::conversation_exists(program, id));
    let resume = wanted
        .filter(|_| !elsewhere)
        .and_then(|_| board.engine.resume_info(name).ok());
    let resumed = resume.is_some();
    let (cmd, cwd) = match resume {
        Some(resume) => resume,
        None => new_session(board, name, existing, alive, elsewhere)?,
    };
    if !recalc::agent::program_exists(&cmd) {
        let agent = board
            .engine
            .agent_of(name)
            .unwrap_or_else(|| "claude".into());
        return Err(recalc::agent::missing_program(&agent));
    }
    let cmd = match &tmux {
        Some(t) => session::wrap(&cmd, t, &cwd),
        None => cmd,
    };
    ptys.spawn(
        app.clone(),
        name.to_string(),
        &cmd,
        &cwd.to_string_lossy(),
        size,
        pty::Mode::Live { tmux },
    )?;
    if !alive {
        board.baseline_session(name);
        if codex && !resumed {
            watch_for_codex_session(
                app,
                board.engine.dir().to_path_buf(),
                CodexId::Cell(name.to_string()),
            );
        }
    }
    board
        .engine
        .set(&recalc::model::status_name(name), "running", Actor::System)
        .map_err(|e| e.to_string())?;
    if elsewhere {
        board
            .engine
            .note_activity(&format!("{name}: {NOT_ON_THIS_COMPUTER}"));
    }
    board::emit_changes(app, board);
    Ok(elsewhere.then(|| NOT_ON_THIS_COMPUTER.to_string()))
}

/// A cell's session command under a conversation id: the live one's, or a
/// new one recorded so any terminal can resume it.
fn new_session(
    board: &mut Board,
    name: &str,
    existing: Option<String>,
    alive: bool,
    // In place of a conversation from another computer: recap what the board kept.
    recap: bool,
) -> Result<(String, std::path::PathBuf), String> {
    let id = match existing {
        Some(id) if alive => id,
        // Agents that choose their own id (Codex) keep the last run's id.
        _ if !board.engine.session_takes_id(name) => uuid::Uuid::new_v4().to_string(),
        _ => {
            let id = uuid::Uuid::new_v4().to_string();
            board
                .engine
                .set_session_id(name, Some(id.clone()))
                .map_err(|e| e.to_string())?;
            id
        }
    };
    let info = if recap {
        board.engine.session_info_recapped(name, &id)
    } else {
        board.engine.session_info(name, &id)
    };
    let (cmd, cwd) = info.ok_or_else(|| format!("{name} is not a runnable agent cell"))?;
    Ok((cmd, cwd))
}

/// Whose Codex conversation a watch records.
pub(crate) enum CodexId {
    Cell(String),
    Driver,
}

/// Codex picks its own conversation id; find it in Codex's session files
/// within a minute and record it on the cell, or as the driver's, so it can
/// be resumed.
pub(crate) fn watch_for_codex_session(app: &AppHandle, dir: std::path::PathBuf, whose: CodexId) {
    let app = app.clone();
    let since = std::time::SystemTime::now() - std::time::Duration::from_secs(2);
    std::thread::spawn(move || {
        for _ in 0..60 {
            std::thread::sleep(std::time::Duration::from_secs(1));
            let state = app.state::<Shared>();
            // Conversations the board already holds aren't this one.
            let taken = {
                let board = state.lock();
                if board.engine.dir() != dir.as_path() {
                    return;
                }
                board.engine.session_ids()
            };
            let Some(id) = session::find_codex_session(&dir, since, &taken) else {
                continue;
            };
            let mut board = state.lock();
            // The window may have switched boards meanwhile.
            if board.engine.dir() == dir.as_path() {
                let _ = match &whose {
                    CodexId::Cell(name) => board.engine.set_session_id(name, Some(id)),
                    CodexId::Driver => board.engine.set_driver_session(&id),
                };
                board::emit_changes(&app, &mut board);
            }
            return;
        }
    });
}

/// End a live session (its tmux session too) and mark its status `ended`.
#[tauri::command]
pub(crate) async fn session_end(
    app: AppHandle,
    state: State<'_, Shared>,
    ptys: State<'_, pty::Ptys>,
    name: String,
) -> Result<(), String> {
    // Board lock first, or a racing open_board could swap boards before
    // "ended" is written.
    let mut board = state.lock();
    let tmux = ptys
        .tmux_of(&name)
        .or_else(|| session::available().then(|| session::name(board.engine.dir(), &name)));
    if let Some(t) = &tmux {
        session::kill(t);
    }
    let was_live = ptys.has_live(&name);
    ptys.kill(&name);
    let status_live = board
        .engine
        .cell(&recalc::model::status_name(&name))
        .is_some_and(|c| matches!(c.value.as_str(), "running" | "waiting?" | "detached"));
    if !was_live && !status_live {
        return Ok(());
    }
    board.session_inputs.remove(&name);
    board
        .engine
        .set(&recalc::model::status_name(&name), "ended", Actor::System)
        .map_err(|e| e.to_string())?;
    board::emit_changes(&app, &mut board);
    Ok(())
}

/// Open a cell's conversation in a desktop terminal (if `open`) and return
/// the command: a tmux attach when live, else the agent's resume command.
#[tauri::command]
pub(crate) async fn session_handoff(
    state: State<'_, Shared>,
    ptys: State<'_, pty::Ptys>,
    name: String,
    open: bool,
) -> Result<String, String> {
    let board = state.lock();
    let cmd = handoff_command(&board, &ptys, &name)?;
    if open {
        session::open_in_terminal(&cmd, terminal_app(&board))?;
    }
    Ok(cmd)
}

/// `[paths] terminal = "iTerm"`: the app that opens handed-off sessions.
pub(crate) fn terminal_app(board: &Board) -> Option<&str> {
    board
        .engine
        .config()
        .paths
        .get("terminal")
        .map(String::as_str)
}

pub(crate) fn handoff_command(
    board: &Board,
    ptys: &pty::Ptys,
    name: &str,
) -> Result<String, String> {
    let tmux = ptys
        .tmux_of(name)
        .or_else(|| session::available().then(|| session::name(board.engine.dir(), name)));
    if let Some(t) = tmux.filter(|t| session::alive(t)) {
        return Ok(session::attach_command(&t));
    }
    if ptys.has_live(name) {
        return Err(
            "this session runs without tmux, so only this window can show it; install tmux (brew install tmux) and start it again to hand it to a terminal"
                .into(),
        );
    }
    let program = board.engine.program_of(name).unwrap_or_default();
    if let Some(id) = board.engine.cell(name).and_then(|c| c.session_id.clone()) {
        if !session::conversation_exists(program, &id) {
            return Err("its last conversation isn't on this computer; open its session here to start a new one, then hand that to a terminal".into());
        }
    }
    // In the board's folder, where the agent worked.
    board.engine.resume_info(name).map(|(cmd, dir)| {
        format!(
            "cd {} && {cmd}",
            recalc::interp::shell_quote(&dir.to_string_lossy())
        )
    })
}

/// On a board switch: mark each live session detached or ended, and drop the clients.
pub(crate) fn detach_all(board: &mut Board, ptys: &pty::Ptys) {
    for (id, detached) in ptys.kill_all() {
        if id == session::DRIVER {
            continue;
        }
        let status = if detached { "detached" } else { "ended" };
        let _ = board
            .engine
            .set(&recalc::model::status_name(&id), status, Actor::System);
    }
}

// ---- the driver: one agent with a seat, no cell ----

/// Start the board's driver, or reattach to it.
#[tauri::command]
pub(crate) async fn driver_open(
    app: AppHandle,
    state: State<'_, Shared>,
    ptys: State<'_, pty::Ptys>,
    cols: u16,
    rows: u16,
    fresh: Option<bool>,
) -> Result<(), String> {
    let board = state.lock();
    if ptys.has_live(session::DRIVER) {
        return Ok(());
    }
    let tmux = session::available().then(|| session::name(board.engine.dir(), session::DRIVER));
    let alive = tmux.as_deref().is_some_and(session::alive);
    // After a restart, continue the driver's conversation unless asked not to.
    let resume = if alive || fresh.unwrap_or(false) {
        None
    } else {
        driver_resume(&board)
    };
    let resumed = resume.is_some();
    let (cmd, cwd) = match resume {
        Some(resume) => resume,
        None => {
            let id = match board.engine.driver_session() {
                Some(id) if alive => id,
                _ => {
                    let id = uuid::Uuid::new_v4().to_string();
                    // Codex picks its own id; it's recorded once found.
                    if board.engine.driver_takes_id() {
                        board
                            .engine
                            .set_driver_session(&id)
                            .map_err(|e| e.to_string())?;
                    }
                    id
                }
            };
            board.engine.driver_info(&id)?
        }
    };
    // Fail with the reason here rather than in a terminal that closes at once.
    if !recalc::agent::program_exists(&cmd) {
        return Err(recalc::agent::missing_program(
            &board.engine.config().driver,
        ));
    }
    let cmd = match &tmux {
        Some(t) => session::wrap(&cmd, t, &cwd),
        None => cmd,
    };
    ptys.spawn(
        app.clone(),
        session::DRIVER.to_string(),
        &cmd,
        &cwd.to_string_lossy(),
        pty::term_size(cols, rows),
        pty::Mode::Live { tmux },
    )?;
    if !alive && !resumed && !board.engine.driver_takes_id() {
        watch_for_codex_session(&app, board.engine.dir().to_path_buf(), CodexId::Driver);
    }
    Ok(())
}

/// The command that continues the driver's last conversation, when there is
/// one recalc can resume.
fn driver_resume(board: &Board) -> Option<(String, std::path::PathBuf)> {
    let id = board.engine.driver_session()?;
    let program = board.engine.driver_program().unwrap_or_default();
    if !session::conversation_exists(program, &id) {
        return None;
    }
    board.engine.driver_resume_info(&id).ok()
}

#[tauri::command]
pub(crate) async fn driver_end(
    state: State<'_, Shared>,
    ptys: State<'_, pty::Ptys>,
) -> Result<(), String> {
    let board = state.lock();
    if session::available() {
        session::kill(&session::name(board.engine.dir(), session::DRIVER));
    }
    ptys.kill(session::DRIVER);
    Ok(())
}

/// Whether the driver runs here, only in its tmux session, or not at all.
#[derive(serde::Serialize)]
#[serde(rename_all = "lowercase")]
enum DriverStatus {
    Running,
    Detached,
    None,
}

/// `running` (attached here), `detached` (alive in tmux, not shown), or
/// `none`; plus which agent has the seat and whether tmux carries it.
#[tauri::command]
pub(crate) async fn driver_status(
    state: State<'_, Shared>,
    ptys: State<'_, pty::Ptys>,
) -> Result<Value, String> {
    let board = state.lock();
    let status = if ptys.has_live(session::DRIVER) {
        DriverStatus::Running
    } else if session::available()
        && session::alive(&session::name(board.engine.dir(), session::DRIVER))
    {
        DriverStatus::Detached
    } else {
        DriverStatus::None
    };
    let resumable = matches!(status, DriverStatus::None) && driver_resume(&board).is_some();
    Ok(json!({
        "status": status,
        "agent": board.engine.config().driver,
        "choices": board.engine.driver_choices(),
        "tmux": session::available(),
        // Start continues the last conversation rather than beginning one.
        "resumable": resumable,
    }))
}

/// Post a desktop notification; the webview decides when.
#[tauri::command]
pub(crate) async fn notify(app: AppHandle, title: String, body: String) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt;
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|e| e.to_string())
}

/// A file dialog's answer, or one queued through the bridge's `POST /pick`.
pub(crate) fn picked(
    dialog: impl FnOnce() -> Option<tauri_plugin_dialog::FilePath>,
) -> Option<tauri_plugin_dialog::FilePath> {
    #[cfg(debug_assertions)]
    if let Some(p) = crate::bridge::take_pick() {
        return Some(tauri_plugin_dialog::FilePath::Path(p));
    }
    dialog()
}

/// Choose the driver agent. Refused while a driver runs.
#[tauri::command]
pub(crate) async fn set_driver(
    app: AppHandle,
    state: State<'_, Shared>,
    ptys: State<'_, pty::Ptys>,
    agent: String,
) -> Result<(), String> {
    let mut board = state.lock();
    let running = ptys.has_live(session::DRIVER)
        || (session::available()
            && session::alive(&session::name(board.engine.dir(), session::DRIVER)));
    if running {
        return Err("End the driver first; the new one takes the seat when you start it".into());
    }
    board
        .engine
        .set_driver(&agent, Actor::Human)
        .map_err(|e| e.to_string())?;
    board::emit_changes(&app, &mut board);
    Ok(())
}

#[tauri::command]
pub(crate) async fn driver_handoff(state: State<'_, Shared>, open: bool) -> Result<String, String> {
    let board = state.lock();
    let name = session::name(board.engine.dir(), session::DRIVER);
    if !(session::available() && session::alive(&name)) {
        return Err("the driver is not running in tmux; start it first".into());
    }
    let cmd = session::attach_command(&name);
    if open {
        session::open_in_terminal(&cmd, terminal_app(&board))?;
    }
    Ok(cmd)
}

/// Type a message into a live session listing its changed inputs. Never
/// automatic: text arriving mid-question could answer it.
#[tauri::command]
pub(crate) async fn session_notify(
    app: AppHandle,
    state: State<'_, Shared>,
    ptys: State<'_, pty::Ptys>,
    name: String,
) -> Result<(), String> {
    let mut board = state.lock();
    let stale = board.stale_inputs(&name);
    if stale.is_empty() {
        return Ok(());
    }
    let changes: Vec<String> = stale
        .iter()
        .map(|(dep, was, now)| format!("{dep} was {:?}, now {:?}", one_line(was), one_line(now)))
        .collect();
    let message = format!(
        "[recalc] Cells this session was given have changed: {}. Re-read them (get_cell) before relying on them.\r",
        changes.join("; ")
    );
    ptys.write(&name, &message)?;
    board.baseline_session(&name);
    board::emit_changes(&app, &mut board);
    Ok(())
}

/// Ask the Driver, in its terminal, to answer agent cells itself rather than
/// run them as agents of their own. The person asked (Ask the Driver to
/// Answer); recalc never types into a terminal on its own. The text goes in,
/// then Enter a moment later: Codex takes keys followed at once by Enter as a
/// paste, and would keep the Enter as a newline.
#[tauri::command]
pub(crate) async fn driver_answer(
    ptys: State<'_, pty::Ptys>,
    names: Vec<String>,
) -> Result<(), String> {
    if !ptys.has_live(session::DRIVER) {
        return Err("the Driver isn't running here; start it from the Driver tab first".into());
    }
    if names.is_empty() {
        return Ok(());
    }
    let message = format!(
        "[recalc] Answer these agent cells yourself, rather than running them as agents of their own: {}. For each, read its prompt and inputs with get_cell, then put your answer in with answer_cell (it keeps the prompt), citing the passage when it rests on a document.",
        cell_list(&names)
    );
    ptys.write(session::DRIVER, &message)?;
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    ptys.write(session::DRIVER, "\r")
}

/// Cells as a person would say them: runs down a column become ranges
/// (`B2:B7, D4`), each sheet's prefix kept.
pub(crate) fn cell_list(names: &[String]) -> String {
    use std::collections::BTreeMap;
    // (sheet prefix, column) -> rows
    let mut cols: BTreeMap<(String, u32), Vec<u32>> = BTreeMap::new();
    let mut other = Vec::new();
    for n in names {
        let (prefix, local) = match n.rfind('!') {
            Some(i) => (n[..=i].to_string(), &n[i + 1..]),
            None => (String::new(), n.as_str()),
        };
        match recalc::model::cell_coord(local) {
            Some((_, c, r)) => cols.entry((prefix, c)).or_default().push(r),
            None => other.push(n.clone()),
        }
    }
    let mut out = Vec::new();
    for ((prefix, col), mut rows) in cols {
        rows.sort_unstable();
        rows.dedup();
        let mut start = rows[0];
        let mut prev = rows[0];
        let mut push = |a: u32, b: u32| {
            let cell = |r| format!("{prefix}{}", recalc::model::a1_name(col, r));
            out.push(if a == b {
                cell(a)
            } else {
                format!("{}:{}", cell(a), recalc::model::a1_name(col, b))
            });
        };
        for &r in &rows[1..] {
            if r != prev + 1 {
                push(start, prev);
                start = r;
            }
            prev = r;
        }
        push(start, prev);
    }
    out.extend(other);
    out.join(", ")
}

/// A value fit for a one-line message: first line, capped.
pub(crate) fn one_line(value: &str) -> String {
    let first = value.lines().next().unwrap_or("");
    if first.chars().count() > 80 {
        let cut: String = first.chars().take(77).collect();
        format!("{cut}...")
    } else {
        first.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::cell_list;

    #[test]
    fn cells_are_listed_as_a_person_would_say_them() {
        let names: Vec<String> = [
            "B3", "B2", "B4", "D4", "B7", "Notes!B2", "Notes!B3", "total",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(cell_list(&names), "B2:B4, B7, D4, Notes!B2:B3, total");
    }
}
