//! The board behind the window: the engine in Tauri state, the pump loop, and
//! the snapshots the webview draws. Mutations are engine calls (`commands`).

use crate::pty;
use recalc::engine::{Axis, Engine, Removed};
use recalc::model::{Actor, Cell, CellKind};
use recalc::store::Watch;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};

pub struct Board {
    pub engine: Engine,
    pub watch: Watch,
    /// ⌘Z stack: each entry takes the board back (on `redo`, forward).
    pub undo: Vec<Undo>,
    pub redo: Vec<Undo>,
    /// Per live session: its prompt's input cells and the values it was given.
    pub session_inputs: HashMap<String, Vec<(String, String)>>,
    /// Made by New Board in this window; left untouched, its folder goes
    /// when the window moves to another board.
    pub fresh: bool,
    /// Reloaded from disk after a panic; the pump redraws the window.
    pub recovered: bool,
}

/// The window's board behind a lock that outlives a panic. A panic while it
/// was held (a bug) would otherwise poison it, and every later command and
/// the engine's pump would fail with it: the window stops answering.
pub struct Shared(Mutex<Board>);

impl Shared {
    pub fn new(board: Board) -> Shared {
        Shared(Mutex::new(board))
    }

    /// The board, never refused. After a panic the engine may be halfway
    /// through a change, so it's reloaded from disk, where every write lands.
    pub fn lock(&self) -> std::sync::MutexGuard<'_, Board> {
        match self.0.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                self.0.clear_poison();
                let mut guard = poisoned.into_inner();
                recover(&mut guard);
                guard
            }
        }
    }
}

/// Replace a board's engine with a fresh one read from disk. Runs in flight
/// are stopped (they'd report to the engine that's gone) and left stale.
fn recover(board: &mut Board) {
    let dir = board.engine.dir().to_path_buf();
    // Release the board first, so the new engine is the one that runs it.
    board.engine.shutdown();
    // Opening starts the engine's runs, which need the async runtime.
    let _rt = tauri::async_runtime::handle().inner().enter();
    match Board::open(&dir) {
        Ok(fresh) => {
            board.engine = fresh.engine;
            board.watch = fresh.watch;
            board.engine.note_activity(
                "recalc hit an internal error and reloaded the board from disk; runs that were going were stopped",
            );
            board.recovered = true;
        }
        Err(e) => eprintln!("recalc: reloading the board after an internal error: {e}"),
    }
}

const UNDO_CAP: usize = 100;

/// One ⌘Z step: what to apply to go back (or, on the redo stack, forward).
/// Applying one pushes its inverse onto the other stack.
#[derive(Clone, Debug)]
pub enum Undo {
    /// Write `content` to the cell. `literal` keeps a value that reads like
    /// a source (`= 1+1` pasted as values) as text.
    Edit {
        name: String,
        content: String,
        literal: bool,
    },
    /// Insert `n` rows or columns at `at` on `sheet` (None: the first) and
    /// restore `removed`: the inverse of a delete.
    Insert {
        sheet: Option<String>,
        axis: Axis,
        at: u32,
        n: u32,
        removed: Removed,
    },
    /// Delete `n` rows or columns at `at` on `sheet`: the inverse of an insert.
    Delete {
        sheet: Option<String>,
        axis: Axis,
        at: u32,
        n: u32,
    },
    /// Move rows on `sheet` by (from, to): the inverse of a sort.
    MoveRows {
        sheet: Option<String>,
        moves: Vec<(u32, u32)>,
    },
    /// One command's steps, undone together. Applied last to first.
    Group(Vec<Undo>),
}

impl Undo {
    /// Whether applying this moves cells to new addresses.
    pub fn is_structural(&self) -> bool {
        match self {
            Undo::Edit { .. } => false,
            Undo::Group(steps) => steps.iter().any(Undo::is_structural),
            _ => true,
        }
    }
}

impl Board {
    pub fn open(dir: &Path) -> Result<Board, String> {
        let mut engine = Engine::open(dir).map_err(|e| e.to_string())?;
        sweep_stale_status(&mut engine);
        let watch = engine.watch().map_err(|e| e.to_string())?;
        Ok(Board {
            engine,
            watch,
            undo: Vec::new(),
            redo: Vec::new(),
            session_inputs: HashMap::new(),
            fresh: false,
            recovered: false,
        })
    }

    /// Record (cell, previous content) pairs as one ⌘Z entry.
    pub fn record_edits(&mut self, entries: impl IntoIterator<Item = (String, String)>) {
        self.record(entries.into_iter().map(|(name, content)| Undo::Edit {
            name,
            content,
            literal: false,
        }));
    }

    /// Push one command's undo steps as one entry, cap the stack, and
    /// invalidate redo.
    pub fn record(&mut self, entries: impl IntoIterator<Item = Undo>) {
        let mut steps: Vec<Undo> = entries.into_iter().collect();
        match steps.len() {
            0 => return,
            1 => self.undo.push(steps.pop().expect("one step")),
            _ => self.undo.push(Undo::Group(steps)),
        }
        if self.undo.len() > UNDO_CAP {
            self.undo.drain(..self.undo.len() - UNDO_CAP);
        }
        self.redo.clear();
    }

    /// The content that would recreate the cell as it is now, in the form
    /// `set` takes.
    pub fn content_of(&self, name: &str) -> String {
        self.engine.content(name).unwrap_or_default()
    }

    /// Remember what a session was given, so drift can be reported later.
    pub fn baseline_session(&mut self, name: &str) {
        let deps = self
            .engine
            .cell(name)
            .map(|c| c.deps.clone())
            .unwrap_or_default();
        let inputs = deps
            .into_iter()
            .map(|d| (d.clone(), self.value_of(&d)))
            .collect();
        self.session_inputs.insert(name.to_string(), inputs);
    }

    /// Inputs of a live session that no longer hold the value it was given.
    pub fn stale_inputs(&self, name: &str) -> Vec<(String, String, String)> {
        self.session_inputs
            .get(name)
            .map(|inputs| {
                inputs
                    .iter()
                    .filter_map(|(dep, was)| {
                        let now = self.value_of(dep);
                        (now != *was).then(|| (dep.clone(), was.clone(), now))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn value_of(&self, name: &str) -> String {
        self.engine
            .cell(name)
            .map(|c| c.value.clone())
            .unwrap_or_default()
    }
}

// ---- snapshots ----

/// The serialized `Cell` plus app-side fields: sparkline, live session, and
/// drifted session inputs.
fn cell_json(board: &Board, sessions: &HashMap<String, pty::SessionStatus>, cell: &Cell) -> Value {
    let mut obj = serde_json::to_value(cell).expect("Cell is serializable");
    let map = obj.as_object_mut().expect("Cell serializes to an object");
    if cell.series {
        let spark: Vec<f64> = board
            .engine
            .history(&cell.name, 40)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(_, v)| v.trim().parse().ok())
            .collect();
        map.insert("spark".into(), json!(spark));
    }
    if let Some(status) = sessions.get(&cell.name) {
        map.insert("session".into(), json!(status.to_string()));
        let stale: Vec<Value> = board
            .stale_inputs(&cell.name)
            .into_iter()
            .map(|(name, was, now)| json!({"name": name, "was": was, "now": now}))
            .collect();
        if !stale.is_empty() {
            map.insert("stale_inputs".into(), json!(stale));
        }
    }
    obj
}

fn envelope(board: &Board, full: bool, cells: Vec<Value>, removed: Vec<String>) -> Value {
    let (running, queued) = board.engine.agent_load();
    let (driver_cost, driver_tokens) = board.engine.driver_usage();
    json!({
        "dir": board.engine.dir().display().to_string(),
        "sheets": board.engine.sheets(),
        "full": full,
        "cells": cells,
        "removed": removed,
        "activity": board.engine.activity().rev().take(50).collect::<Vec<_>>(),
        "agents": {
            "running": running,
            "queued": queued,
            "max": board.engine.config().max_agents.max(1),
            "paused_until": board.engine.agents_paused_until(),
            "budget": board.engine.config().budget_usd,
            "spent": board.engine.spent(),
            "driver_cost": driver_cost,
            "driver_tokens": driver_tokens,
        },
    })
}

/// Every cell, for boot and board switches.
pub fn full_snapshot(board: &Board, ptys: &pty::Ptys) -> Value {
    let sessions = ptys.live_statuses();
    let cells = board
        .engine
        .all_cells()
        .map(|c| cell_json(board, &sessions, c))
        .collect();
    envelope(board, true, cells, Vec::new())
}

/// Only the named cells, changed or removed.
pub fn delta_snapshot(board: &Board, ptys: &pty::Ptys, names: &HashSet<String>) -> Value {
    let sessions = ptys.live_statuses();
    let mut cells = Vec::new();
    let mut removed = Vec::new();
    for name in names {
        match board.engine.cell(name) {
            Some(c) => cells.push(cell_json(board, &sessions, c)),
            None => removed.push(name.clone()),
        }
    }
    envelope(board, false, cells, removed)
}

/// Push the cells changed since the last push, if any.
pub fn emit_changes(app: &AppHandle, board: &mut Board) {
    let changed = board.engine.take_changed();
    if changed.is_empty() {
        return;
    }
    let ptys = app.state::<pty::Ptys>();
    let _ = app.emit("board", delta_snapshot(board, &ptys, &changed));
}

/// Like `emit_changes`, but always sends: board status (such as the agent
/// budget) rides on every snapshot and may change with no cell changing.
pub fn emit_status(app: &AppHandle, board: &mut Board) {
    let changed = board.engine.take_changed();
    let ptys = app.state::<pty::Ptys>();
    let _ = app.emit("board", delta_snapshot(board, &ptys, &changed));
}

pub fn emit_full(app: &AppHandle, board: &mut Board) {
    board.engine.take_changed(); // the full snapshot covers them
    let ptys = app.state::<pty::Ptys>();
    let _ = app.emit("board", full_snapshot(board, &ptys));
}

// ---- templates ----

/// The example boards, embedded in the binary.
pub const TEMPLATES: &[(&str, &str)] = &[
    (
        "welcome",
        include_str!("../../../examples/welcome.recalctemplate"),
    ),
    (
        "review",
        include_str!("../../../examples/review.recalctemplate"),
    ),
    (
        "reviewer",
        include_str!("../../../examples/reviewer.recalctemplate"),
    ),
    (
        "verify",
        include_str!("../../../examples/verify.recalctemplate"),
    ),
    (
        "mission",
        include_str!("../../../examples/mission.recalctemplate"),
    ),
    (
        "batch",
        include_str!("../../../examples/batch.recalctemplate"),
    ),
    (
        "eval",
        include_str!("../../../examples/eval.recalctemplate"),
    ),
    (
        "multiagent",
        include_str!("../../../examples/multiagent.recalctemplate"),
    ),
    (
        "handoff",
        include_str!("../../../examples/handoff.recalctemplate"),
    ),
];

/// A template's leading comment lines, joined and cut after the first sentence.
pub fn template_note(text: &str) -> String {
    let joined = text
        .lines()
        .map_while(|l| l.strip_prefix('#'))
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    match joined.find(". ") {
        Some(i) => joined[..i + 1].to_string(),
        None => joined,
    }
}

// ---- recents ----

/// Where Tauri's `app_config_dir` is, computed without an app handle so the
/// board can be chosen before the window exists.
fn recents_path() -> Option<PathBuf> {
    Some(config_dir()?.join("recents.json"))
}

/// The app's own settings folder (Tauri's `app_config_dir`).
pub fn config_dir() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join(IDENTIFIER))
}

const IDENTIFIER: &str = "dev.recalc.desktop";

pub fn load_recents() -> Vec<String> {
    recents_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn push_recent(dir: &Path) {
    // A window driven over the bridge is a test; keep it out of the person's list.
    #[cfg(debug_assertions)]
    if crate::bridge::port().is_some() {
        return;
    }
    let dir = dir.display().to_string();
    let mut recents = load_recents();
    recents.retain(|r| r != &dir);
    recents.insert(0, dir);
    recents.truncate(8);
    if let Some(path) = recents_path() {
        let _ = std::fs::create_dir_all(path.parent().expect("has parent"));
        let _ = std::fs::write(path, serde_json::to_string(&recents).expect("strings"));
    }
}

/// Whether this binary runs from a disk image's root (`/Volumes/<image>/recalc.app`)
/// or a copy macOS made to run a downloaded app in place (App Translocation).
pub(crate) fn from_disk_image(exe: &Path) -> bool {
    let s = exe.to_string_lossy();
    if s.contains("/AppTranslocation/") {
        return true;
    }
    let parts: Vec<_> = exe.components().collect();
    // ["/", "Volumes", "<image>", "recalc.app", ...]
    parts.len() > 4
        && parts[1].as_os_str() == "Volumes"
        && parts[3].as_os_str().to_string_lossy().ends_with(".app")
}

/// The board to open when none was named. Opened from Finder or the Dock the
/// working directory is `/` (a Linux launcher gives home), never a board, so
/// reopen the most recent board, or else `~/recalc`.
pub fn launch_dir() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    if cwd != Path::new("/") && cwd != home {
        return cwd;
    }
    load_recents()
        .into_iter()
        .map(PathBuf::from)
        .find(|d| d.join(".recalc").is_dir())
        .unwrap_or_else(|| home.join("recalc"))
}

/// On open, a persisted status claiming a live session becomes `detached`
/// if its tmux session survives, `ended` if not.
fn sweep_stale_status(engine: &mut Engine) {
    // Each agent cell's status cell, found from the cell (`status_name`).
    let stale: Vec<(String, bool)> = engine
        .all_cells()
        .filter(|c| c.kind == CellKind::AgentCall)
        .filter(|owner| {
            engine
                .cell(&recalc::model::status_name(&owner.name))
                .is_some_and(|s| matches!(s.value.as_str(), "running" | "waiting?" | "detached"))
        })
        .map(|owner| {
            let alive = recalc::session::available()
                && recalc::session::alive(&recalc::session::name(engine.dir(), &owner.name));
            (recalc::model::status_name(&owner.name), alive)
        })
        .collect();
    for (name, alive) in stale {
        let _ = engine.set(
            &name,
            if alive { "detached" } else { "ended" },
            Actor::System,
        );
    }
}

/// Every 5 s: a `detached` session whose tmux session is gone has ended.
fn sweep_ended_sessions(engine: &mut Engine, ptys: &pty::Ptys) {
    if !recalc::session::available() {
        return;
    }
    let ended: Vec<String> = engine
        .all_cells()
        .filter(|owner| {
            engine
                .cell(&recalc::model::status_name(&owner.name))
                .is_some_and(|s| s.value == "detached")
        })
        .map(|owner| owner.name.clone())
        .filter(|owner| !ptys.has_live(owner))
        .filter(|owner| !recalc::session::alive(&recalc::session::name(engine.dir(), owner)))
        .collect();
    for owner in ended {
        let _ = engine.set(&recalc::model::status_name(&owner), "ended", Actor::System);
    }
}

/// One pass of the window's pump: session transitions, the MCP server's
/// window requests, file changes and finished runs, then a delta to the page.
fn pump_pass(handle: &AppHandle, sweep_tick: &mut u32) {
    let ptys = handle.state::<pty::Ptys>();
    let state = handle.state::<Shared>();
    let mut guard = state.lock();
    // Poll under the Board lock (order: Board, then Ptys), or a
    // stale "running" could overwrite a session_end's "ended".
    let transitions = ptys.poll();
    let board = &mut *guard;
    let mut extra: HashSet<String> = HashSet::new();
    // Transitions become ordinary `<cell>_status` cells.
    for (cell, status) in transitions {
        if cell == recalc::session::DRIVER {
            continue; // the driver has a seat, not a cell
        }
        let _ = board.engine.set(
            &recalc::model::status_name(&cell),
            &status.to_string(),
            Actor::System,
        );
        if matches!(status, pty::SessionStatus::Done(_)) {
            board.session_inputs.remove(&cell);
        }
        extra.insert(cell);
    }
    // Window requests the MCP server left as files.
    for (kind, name) in board.engine.take_requests() {
        match kind.as_str() {
            "open_session" => {
                let size = pty::term_size(100, 30);
                match crate::commands::start_session(handle, board, &ptys, &name, size, false) {
                    Ok(_) => {
                        let _ = handle.emit("show-session", name.clone());
                    }
                    Err(e) => {
                        board.engine.note_activity(&format!(
                            "an agent asked to open {name}'s session: {e}"
                        ));
                        // The asking agent waits on this cell.
                        let _ = board.engine.set(
                            &recalc::model::status_name(&name),
                            &format!("error: {e}"),
                            Actor::System,
                        );
                    }
                }
            }
            "show_cell" => {
                let _ = handle.emit("show-cell", name.clone());
            }
            _ => {}
        }
    }
    // A detached agent can exit unobserved; ask tmux.
    *sweep_tick = (*sweep_tick + 1) % 100;
    if *sweep_tick == 0 {
        sweep_ended_sessions(&mut board.engine, &ptys);
    }
    let _ = board.engine.pump(&mut board.watch);
    let mut changed = board.engine.take_changed();
    // A session whose input moved must redraw its drift note.
    for (sess, inputs) in &board.session_inputs {
        if inputs.iter().any(|(dep, _)| changed.contains(dep)) {
            extra.insert(sess.clone());
        }
    }
    changed.extend(extra);
    if board.engine.take_news() || !changed.is_empty() {
        let _ = handle.emit("board", delta_snapshot(board, &ptys, &changed));
    }
}

/// Title the window with the board's directory name.
pub fn set_title(app: &AppHandle, dir: &Path) {
    // A package's name without its `.recalc`, as Finder shows it.
    let name = match dir.extension() {
        Some(e) if e == "recalc" => dir.file_stem(),
        _ => dir.file_name(),
    }
    .map(|n| n.to_string_lossy().into_owned())
    .unwrap_or_else(|| dir.display().to_string());
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.set_title(&format!("{name} — recalc beta"));
    }
}

/// Set once setup has opened the first board.
static SET_UP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Open the board in `dir` and run the app until the window closes.
pub fn run(dir: PathBuf) {
    // Launched from the Dock, PATH is launchd's; take the login shell's first.
    recalc::shell_env::adopt_login_shell_path();
    recalc::shell_env::put_own_dir_on_path();
    // Opening a board can spawn runs, which need the runtime.
    let board = tauri::async_runtime::block_on(async { Board::open(&dir) });
    let board = match board {
        Ok(b) => b,
        Err(e) => {
            eprintln!("recalc: {e}");
            std::process::exit(1);
        }
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(Shared::new(board))
        .manage(crate::update::Pending::default())
        .manage(pty::Ptys::default())
        .menu(crate::menu::build)
        .on_menu_event(|app, event| {
            let _ = app.emit("menu", event.id().0.clone());
        })
        .setup(move |app| {
            #[cfg(debug_assertions)]
            if let Some(port) = crate::bridge::port() {
                crate::bridge::start(app.handle(), port);
            }
            push_recent(&dir);
            set_title(app.handle(), &dir);
            if let Some(window) = app.get_webview_window("main") {
                crate::native::native_toolbar(&window);
            }
            // Killed rather than quit (SIGTERM, a closed terminal): end the
            // runs first; they live in their own process groups.
            tauri::async_runtime::spawn(recalc::exec::exit_on_signal());

            let handle = app.handle().clone();
            // Pump loop: absorb file changes and finished runs, push deltas.
            tauri::async_runtime::spawn(async move {
                let mut tick = tokio::time::interval(std::time::Duration::from_millis(50));
                let mut sweep_tick = 0u32;
                loop {
                    tick.tick().await;
                    // A panic in one pass (a bug) mustn't end the pump: the next
                    // pass's lock reloads the board, and the window keeps working.
                    let pass = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        pump_pass(&handle, &mut sweep_tick)
                    }));
                    if pass.is_err() {
                        eprintln!(
                            "recalc: the engine's pump hit an internal error; reloading the board"
                        );
                    }
                }
            });
            crate::update::start_checks(app.handle());
            SET_UP.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        })
        .invoke_handler(crate::commands::handler())
        .build(tauri::generate_context!())
        .expect("error while running recalc")
        .run(|app, event| match event {
            // Tauri exits without dropping managed state; stop runs here.
            tauri::RunEvent::Exit => {
                app.state::<Shared>().lock().engine.shutdown();
            }
            // A `.recalc` double-clicked in Finder or dropped on the Dock icon.
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Opened { urls } => {
                for path in urls.iter().filter_map(|u| u.to_file_path().ok()) {
                    // A folder's URL ends in a slash; recents shouldn't keep it.
                    let path: PathBuf = path.components().collect();
                    let app = app.clone();
                    // Off the event loop: opening a file may show a dialog.
                    std::thread::spawn(move || {
                        // Launched by the double-click, the event comes before setup
                        // has opened the first board; open this one after it.
                        for _ in 0..200 {
                            if SET_UP.load(std::sync::atomic::Ordering::SeqCst) {
                                break;
                            }
                            std::thread::sleep(std::time::Duration::from_millis(50));
                        }
                        let (Some(state), Some(ptys)) =
                            (app.try_state::<Shared>(), app.try_state::<pty::Ptys>())
                        else {
                            return;
                        };
                        if let Err(e) = crate::commands::open_path(&app, &state, &ptys, &path) {
                            let name = path.file_name().map_or_else(
                                || path.display().to_string(),
                                |n| n.to_string_lossy().into_owned(),
                            );
                            let _ = app.emit("notice", format!("{name}: {e}"));
                        }
                    });
                }
            }
            _ => {}
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_app_run_from_its_disk_image_is_told_to_move() {
        let yes = [
            "/Volumes/recalc/recalc.app/Contents/MacOS/recalc",
            "/private/var/folders/x/T/AppTranslocation/ABC/d/recalc.app/Contents/MacOS/recalc",
        ];
        let no = [
            "/Applications/recalc.app/Contents/MacOS/recalc",
            "/Volumes/External/Applications/recalc.app/Contents/MacOS/recalc",
            "/Users/me/recalc/target/debug/recalc",
        ];
        assert!(yes.iter().all(|p| from_disk_image(Path::new(p))));
        assert!(!no.iter().any(|p| from_disk_image(Path::new(p))));
    }

    #[test]
    fn recents_live_where_tauri_keeps_app_config() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(conf["identifier"], IDENTIFIER);
    }

    fn board() -> (tempfile::TempDir, Board) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".recalc")).unwrap(); // not a first run
        let board = Board::open(dir.path()).unwrap();
        (dir, board)
    }

    #[test]
    fn a_panic_holding_the_board_doesnt_lock_the_window_out() {
        let (_d, board) = board();
        let shared = std::sync::Arc::new(Shared::new(board));
        shared.lock().engine.set("A1", "1", Actor::Human).unwrap();
        // A bug panics while the board is held: the lock is poisoned.
        let held = shared.clone();
        let _ = std::thread::spawn(move || {
            let _board = held.lock();
            panic!("a bug");
        })
        .join();
        // The next caller gets the board, reloaded from disk, and it works.
        let mut board = shared.lock();
        assert!(
            std::mem::take(&mut board.recovered),
            "the window is told to redraw"
        );
        assert_eq!(board.engine.cell("A1").unwrap().value, "1");
        assert!(board
            .engine
            .activity()
            .any(|a| a.text.contains("reloaded the board from disk")));
        board.engine.set("A2", "= {A1} + 1", Actor::Human).unwrap();
        assert_eq!(board.engine.cell("A2").unwrap().value, "2");
        drop(board);
        // And it stays unpoisoned.
        assert!(!shared.lock().recovered);
    }

    #[tokio::test]
    async fn snapshot_carries_what_the_ui_draws() {
        let (_d, mut board) = board();
        board.engine.set("failed", "0", Actor::Human).unwrap();
        board
            .engine
            .set("status", "= {failed} == 0", Actor::Human)
            .unwrap();
        board.engine.set_series("failed", true).unwrap();
        board.engine.set("failed", "2", Actor::Human).unwrap();

        let snap = full_snapshot(&board, &pty::Ptys::default());
        assert_eq!(snap["full"], true);
        assert!(snap["dir"].is_string());
        assert_eq!(snap["agents"]["max"], 4);
        assert!(snap["activity"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["text"].as_str().unwrap().contains("set status")
                && e["by"] == "human"
                && e["at"].as_u64().unwrap() > 0));

        let cells = snap["cells"].as_array().unwrap();
        let status = cells.iter().find(|c| c["name"] == "status").unwrap();
        assert_eq!(status["value"], "false");
        let failed = cells.iter().find(|c| c["name"] == "failed").unwrap();
        assert_eq!(failed["spark"], json!([0.0, 2.0]));
    }

    #[tokio::test]
    async fn delta_names_changed_and_removed_cells() {
        let (_d, mut board) = board();
        board.engine.set("a", "1", Actor::Human).unwrap();
        board.engine.set("b", "= {a} + 1", Actor::Human).unwrap();
        board.engine.take_changed();
        board.engine.set("a", "5", Actor::Human).unwrap();
        board
            .engine
            .delete_many(&["b".to_string()], Actor::Human)
            .unwrap();
        let changed = board.engine.take_changed();
        let snap = delta_snapshot(&board, &pty::Ptys::default(), &changed);
        assert_eq!(snap["full"], false);
        let names: Vec<&str> = snap["cells"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["a"]);
        assert_eq!(snap["removed"], json!(["b"]));
    }

    #[tokio::test]
    async fn session_inputs_report_drift() {
        let (_d, mut board) = board();
        board.engine.set("A2", "doc-a", Actor::Human).unwrap();
        board
            .engine
            .set("agent", "@claude('read {A2}')", Actor::Human)
            .unwrap();
        board.baseline_session("agent");
        assert!(board.stale_inputs("agent").is_empty());
        board.engine.set("A2", "doc-b", Actor::Human).unwrap();
        assert_eq!(
            board.stale_inputs("agent"),
            [("A2".to_string(), "doc-a".to_string(), "doc-b".to_string())]
        );
        board.baseline_session("agent");
        assert!(board.stale_inputs("agent").is_empty());
    }

    #[test]
    fn a_fresh_board_opens_empty() {
        let dir = tempfile::tempdir().unwrap();
        let board = Board::open(dir.path()).unwrap();
        assert_eq!(board.engine.all_cells().count(), 0);
    }

    /// types.ts's hand-written `Cell` must match the serialized fields.
    #[test]
    fn the_webview_cell_type_names_every_serialized_field() {
        let cell = serde_json::to_value(Cell::new("x")).unwrap();
        let serialized: std::collections::BTreeSet<String> =
            cell.as_object().unwrap().keys().cloned().collect();
        let ts = include_str!("../../frontend/src/types.ts");
        let start = ts
            .find("export interface Cell {")
            .expect("types.ts declares Cell");
        let body = &ts[start..];
        let end = body
            .find("// grafted on by the snapshot")
            .expect("the snapshot's extra fields are marked");
        let declared: std::collections::BTreeSet<String> = body[..end]
            .lines()
            .skip(1)
            .filter_map(|l| l.trim().split_once(':'))
            .map(|(k, _)| k.trim().trim_end_matches('?').to_string())
            .collect();
        assert_eq!(
            serialized, declared,
            "types.ts's Cell and the engine's serialized Cell differ"
        );
    }

    #[tokio::test]
    async fn every_template_parses() {
        for (name, text) in TEMPLATES {
            let dir = tempfile::tempdir().unwrap();
            let mut e = Engine::open(dir.path()).unwrap();
            e.apply_template(text, Actor::Cli).unwrap();
            assert_eq!(
                e.source_problems(),
                Vec::<(String, String)>::new(),
                "template {name}"
            );
        }
    }

    #[test]
    fn template_notes_are_the_opening_comment_to_its_first_sentence() {
        assert!(template_note(TEMPLATES[0].1).starts_with("Welcome"));
        assert_eq!(template_note("A1 = 1\n"), "");
        assert_eq!(
            template_note("# Batch board: one task across a column of\n# inputs, with per-row status. Edit an input and\nA1 = 1\n"),
            "Batch board: one task across a column of inputs, with per-row status."
        );
        for (name, text) in TEMPLATES {
            let note = template_note(text);
            assert!(
                note.ends_with('.') || !note.contains(". "),
                "{name}: {note}"
            );
        }
    }
}
