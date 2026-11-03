//! Terminal cells: a PTY per cell, streamed to xterm.js in the webview.
//!
//! - A plain view (`pty_open`) runs a `$`/`!` cell's command and dies with
//!   its panel.
//! - A live agent session (`session_open`) belongs to an `@` cell, survives
//!   the panel closing, buffers output for reattach, and reports status
//!   transitions that the caller writes to `<cell>_status`.
//!
//! With tmux on PATH a live session runs inside tmux (`recalc::session`) and
//! our PTY is one client of it, so the agent outlives the window.
//!
//! PTYs never write cell values; that is the job of non-interactive runs and
//! MCP `set_cell`.

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::{AppHandle, Emitter, State};

/// Output silence this long, ending on a prompt, reads as "waiting?".
/// Best-effort; the reliable signal is the agent calling `ask_cell`.
const IDLE_BEFORE_WAITING: std::time::Duration = std::time::Duration::from_secs(5);
/// Reattach scrollback kept per live session.
const BUFFER_CAP: usize = 200_000;

struct Session {
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    /// Present for live agent sessions; shared with the reader thread.
    live: Option<Arc<Mutex<Live>>>,
}

struct Live {
    buffer: String,
    last_output: Instant,
    /// Last status reported by `poll`, so only transitions are emitted.
    status: SessionStatus,
    /// The tmux session this PTY is a client of; our child exiting then
    /// means detached unless the session is gone too.
    tmux: Option<String>,
}

#[derive(Default)]
pub struct Ptys(Mutex<HashMap<String, Session>>);

/// A plain view dies with its panel; a live session outlives it.
pub enum Mode {
    Plain,
    Live { tmux: Option<String> },
}

#[derive(Clone, Serialize)]
struct Chunk {
    id: String,
    data: String,
}

#[derive(Clone, Serialize)]
struct Exit {
    id: String,
}

pub(crate) fn term_size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

/// TERM and a UTF-8 locale for every PTY: an app launched from Finder
/// inherits neither, tmux won't start without TERM, and without a locale it
/// draws non-ASCII as underscores.
fn terminal_env(cmd: &mut CommandBuilder) {
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    let has_locale = ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .any(|k| std::env::var(k).is_ok_and(|v| !v.is_empty()));
    if !has_locale {
        cmd.env("LANG", "en_US.UTF-8");
    }
}

/// A live session's state, written to `<cell>_status` as its `Display` text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStatus {
    Running,
    /// Idle at what looks like the agent's prompt.
    Waiting,
    /// The terminal closed but the tmux session lives on.
    Detached,
    Done(u32),
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionStatus::Running => f.write_str("running"),
            SessionStatus::Waiting => f.write_str("waiting?"),
            SessionStatus::Detached => f.write_str("detached"),
            SessionStatus::Done(code) => write!(f, "done (exit {code})"),
        }
    }
}

impl Ptys {
    pub(crate) fn spawn<R: tauri::Runtime>(
        &self,
        app: AppHandle<R>,
        id: String,
        command: &str,
        cwd: &str,
        size: PtySize,
        mode: Mode,
    ) -> Result<(), String> {
        self.kill(&id);

        let pair = native_pty_system()
            .openpty(size)
            .map_err(|e| e.to_string())?;
        let mut cmd = CommandBuilder::new("sh");
        cmd.arg("-c");
        cmd.arg(command);
        cmd.cwd(cwd);
        terminal_env(&mut cmd);
        let child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
        // The slave is held by the child; drop our handle so EOF arrives on exit.
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer = pair.master.take_writer().map_err(|e| e.to_string())?;

        let live = match mode {
            Mode::Plain => None,
            Mode::Live { tmux } => Some(Arc::new(Mutex::new(Live {
                buffer: String::new(),
                last_output: Instant::now(),
                status: SessionStatus::Running,
                tmux,
            }))),
        };
        let live_reader = live.clone();
        let emit_id = id.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let data = String::from_utf8_lossy(&buf[..n]).into_owned();
                        if let Some(live) = &live_reader {
                            let mut l = live.lock().unwrap();
                            l.last_output = Instant::now();
                            l.buffer.push_str(&data);
                            if l.buffer.len() > BUFFER_CAP {
                                let mut cut = l.buffer.len() - BUFFER_CAP;
                                while !l.buffer.is_char_boundary(cut) {
                                    cut += 1;
                                }
                                l.buffer.drain(..cut);
                            }
                        }
                        if app
                            .emit(
                                "pty-data",
                                Chunk {
                                    id: emit_id.clone(),
                                    data,
                                },
                            )
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
            let _ = app.emit("pty-exit", Exit { id: emit_id });
        });

        self.0.lock().unwrap().insert(
            id,
            Session {
                writer,
                master: pair.master,
                child,
                live,
            },
        );
        Ok(())
    }

    /// Type into a session, as the human at its keyboard would.
    pub fn write(&self, id: &str, data: &str) -> Result<(), String> {
        let mut sessions = self.0.lock().unwrap();
        let session = sessions.get_mut(id).ok_or("no such terminal")?;
        session
            .writer
            .write_all(data.as_bytes())
            .map_err(|e| e.to_string())?;
        session.writer.flush().map_err(|e| e.to_string())
    }

    pub(crate) fn kill(&self, id: &str) {
        let session = self.0.lock().unwrap().remove(id);
        if let Some(mut session) = session {
            let _ = session.child.kill();
            let _ = session.child.wait(); // reap; kill alone leaves a zombie
        }
    }

    /// Kill every client (board switch). Returns each live id and whether its
    /// tmux session survives (detached) or not (ended).
    pub fn kill_all(&self) -> Vec<(String, bool)> {
        let sessions = std::mem::take(&mut *self.0.lock().unwrap());
        let mut live = Vec::new();
        for (id, mut s) in sessions {
            let _ = s.child.kill();
            let _ = s.child.wait();
            if let Some(l) = &s.live {
                let tmux = l.lock().unwrap().tmux.clone();
                let detached = tmux.as_deref().is_some_and(recalc::session::alive);
                live.push((id, detached));
            }
        }
        live
    }

    /// The tmux session behind a live id, if it runs in one.
    pub fn tmux_of(&self, id: &str) -> Option<String> {
        self.0
            .lock()
            .unwrap()
            .get(id)
            .and_then(|s| s.live.as_ref())
            .and_then(|l| l.lock().unwrap().tmux.clone())
    }

    pub fn has_live(&self, id: &str) -> bool {
        // An exited session stays in the table until `poll` reaps it, so ask the process.
        self.0
            .lock()
            .unwrap()
            .get_mut(id)
            .is_some_and(|s| s.live.is_some() && matches!(s.child.try_wait(), Ok(None)))
    }

    /// Current status per live session, for the board snapshot.
    pub fn live_statuses(&self) -> HashMap<String, SessionStatus> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(id, s)| {
                let live = s.live.as_ref()?;
                Some((id.clone(), live.lock().unwrap().status))
            })
            .collect()
    }

    /// Status transitions since the last poll: `(cell, status)` pairs the
    /// caller writes to `<cell>_status`. Exited sessions are reaped here.
    pub fn poll(&self) -> Vec<(String, SessionStatus)> {
        let mut out = Vec::new();
        let mut done = Vec::new();
        let mut map = self.0.lock().unwrap();
        for (id, s) in map.iter_mut() {
            let Some(live) = &s.live else { continue };
            if let Ok(Some(status)) = s.child.try_wait() {
                // A tmux client exiting while its session lives was detached.
                let tmux = live.lock().unwrap().tmux.clone();
                let status = if tmux.as_deref().is_some_and(recalc::session::alive) {
                    SessionStatus::Detached
                } else {
                    SessionStatus::Done(status.exit_code())
                };
                out.push((id.clone(), status));
                done.push(id.clone());
                continue;
            }
            let mut l = live.lock().unwrap();
            let idle = l.last_output.elapsed() >= IDLE_BEFORE_WAITING;
            let next = if idle && looks_idle(&l.buffer) {
                SessionStatus::Waiting
            } else {
                SessionStatus::Running
            };
            if l.status != next {
                l.status = next;
                out.push((id.clone(), next));
            }
        }
        for id in done {
            map.remove(&id);
        }
        out
    }
}

/// The last non-empty line of the buffer, ANSI escapes stripped.
#[cfg(test)]
fn last_line(buffer: &str) -> String {
    let plain = strip_ansi(buffer);
    plain
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string()
}

/// Drop ESC-sequences and control characters; keep printable text.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            match chars.peek() {
                // CSI: ESC [ ... final byte @-~
                Some('[') => {
                    chars.next();
                    for c in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&c) {
                            break;
                        }
                    }
                }
                // OSC: ESC ] ... BEL or ESC \ (consume the ST's backslash)
                Some(']') => {
                    chars.next();
                    for c in chars.by_ref() {
                        if c == '\u{7}' {
                            break;
                        }
                        if c == '\u{1b}' {
                            chars.next_if_eq(&'\\');
                            break;
                        }
                    }
                }
                _ => {
                    chars.next();
                }
            }
        } else if c == '\n' || c == '\r' {
            // \r rewrites the line, so treat it as a line break.
            out.push('\n');
        } else if !c.is_control() {
            out.push(c);
        }
    }
    out
}

/// Whether the agent sits at its prompt. Agent TUIs draw a footer below the
/// input line, so the last few lines are checked, not just the last; a
/// "working" line outranks a prompt.
fn looks_idle(buffer: &str) -> bool {
    let plain = strip_ansi(buffer);
    let tail: Vec<&str> = plain
        .lines()
        .rev()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .take(8)
        .collect();
    if tail.iter().any(|l| {
        let lower = l.to_ascii_lowercase();
        lower.contains("esc to interrupt") || lower.contains("working (")
    }) {
        return false;
    }
    tail.iter()
        .any(|l| looks_like_prompt(l) || l.starts_with('›'))
}

/// Whether a line reads like a question. Deliberately loose: it only sets
/// the "waiting?" status.
fn looks_like_prompt(line: &str) -> bool {
    let l = line.trim();
    let lower = l.to_ascii_lowercase();
    l.ends_with('?')
        || l.ends_with(':')
        || l.starts_with('❯')
        || l.starts_with('>')
        || lower.contains("(y/n)")
        || lower.contains("[y/n]")
        || lower.contains("press enter")
}

/// Open a PTY running `command` (via `sh -c`) in `cwd`, keyed by `id` (the
/// cell name). Output streams as `pty-data` events; termination as `pty-exit`.
/// Reopening an id replaces the previous session.
#[tauri::command]
pub fn pty_open(
    app: AppHandle,
    ptys: State<'_, Ptys>,
    id: String,
    command: String,
    cwd: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    ptys.spawn(app, id, &command, &cwd, term_size(cols, rows), Mode::Plain)
}

#[tauri::command]
pub fn pty_write(ptys: State<'_, Ptys>, id: String, data: String) -> Result<(), String> {
    ptys.write(&id, &data)
}

#[tauri::command]
pub fn pty_resize(ptys: State<'_, Ptys>, id: String, cols: u16, rows: u16) -> Result<(), String> {
    let sessions = ptys.0.lock().unwrap();
    match sessions.get(&id) {
        Some(session) => session
            .master
            .resize(term_size(cols, rows))
            .map_err(|e| e.to_string()),
        None => Ok(()), // resize can race a close; nothing to do
    }
}

#[tauri::command]
pub fn pty_close(ptys: State<'_, Ptys>, id: String) {
    ptys.kill(&id);
}

/// Reattach scrollback for a live session (empty if none).
#[tauri::command]
pub fn session_buffer(ptys: State<'_, Ptys>, id: String) -> String {
    ptys.0
        .lock()
        .unwrap()
        .get(&id)
        .and_then(|s| s.live.as_ref())
        .map(|l| l.lock().unwrap().buffer.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_tui_at_its_prompt_reads_as_idle_and_one_working_does_not() {
        use super::looks_idle;
        let claude_idle =
            "  Files touched: only reports/checkout-lift.md. I ran no git add or commit.\n\
✻ Brewed for 45s · done 9:53 PM\n\
────────────────────────────────────────\n\
❯ \n\
────────────────────────────────────────\n\
  ⏵⏵ auto mode on (shift+tab to cycle) · ← for agents\n";
        assert!(looks_idle(claude_idle), "the prompt sits above the footer");
        let claude_working = "  Reading reports/latency-p50.md\n\
────────────────────────────────────────\n\
❯ \n\
────────────────────────────────────────\n\
  ✻ Sautéing… (esc to interrupt)\n";
        assert!(
            !looks_idle(claude_working),
            "a working line outranks the prompt"
        );
        let codex_idle =
            "  Done: edited the report.\n\n› \n\n  100% context left · ? for shortcuts\n";
        assert!(looks_idle(codex_idle));
        let plain_shell = "$ cargo test\nrunning 3 tests\ntest a ... ok\n";
        assert!(
            !looks_idle(plain_shell),
            "output that is not a prompt is not idle"
        );
        let bare_prompt = "some output\n$ ";
        assert!(
            !looks_idle(bare_prompt),
            "a shell's $ is not one of the agents' prompts"
        );
    }

    use super::*;

    // A TTY-backed command echoes input and closes on exit.
    #[test]
    fn pty_round_trips_input_and_closes() {
        let pair = native_pty_system().openpty(term_size(80, 24)).unwrap();
        let mut cmd = CommandBuilder::new("sh");
        cmd.arg("-c");
        cmd.arg("read line; echo \"got:$line\"");
        cmd.cwd(std::env::temp_dir());
        let mut child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().unwrap();
        let mut writer = pair.master.take_writer().unwrap();
        writer.write_all(b"hello\n").unwrap();
        writer.flush().unwrap();

        let mut out = String::new();
        let mut buf = [0u8; 1024];
        // Read until the child exits (reader hits EOF once the slave closes).
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => out.push_str(&String::from_utf8_lossy(&buf[..n])),
                Err(_) => break,
            }
        }
        child.wait().unwrap();
        assert!(out.contains("got:hello"), "pty did not echo input: {out:?}");
    }

    #[test]
    fn live_session_buffers_and_reports_exit() {
        let app = tauri::test::mock_app();
        let ptys = Ptys::default();
        ptys.spawn(
            app.handle().clone(),
            "review".into(),
            // Long enough to still be running when it's checked below.
            "echo session-output; sleep 0.5",
            &std::env::temp_dir().to_string_lossy(),
            term_size(80, 24),
            Mode::Live { tmux: None },
        )
        .unwrap();
        assert!(ptys.has_live("review"));
        assert_eq!(ptys.live_statuses()["review"], SessionStatus::Running);

        // A "waiting?" transition may come first on a loaded machine.
        let deadline = Instant::now() + std::time::Duration::from_secs(20);
        loop {
            let transitions = ptys.poll();
            if let Some((id, status)) = transitions
                .iter()
                .find(|(_, s)| matches!(s, SessionStatus::Done(_)))
            {
                assert_eq!(id, "review");
                assert_eq!(*status, SessionStatus::Done(0));
                break;
            }
            assert!(Instant::now() < deadline, "session never exited");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(!ptys.has_live("review"));
    }

    #[test]
    fn session_buffer_replays_output() {
        let app = tauri::test::mock_app();
        let ptys = Ptys::default();
        ptys.spawn(
            app.handle().clone(),
            "s".into(),
            "printf 'hello '; sleep 5",
            &std::env::temp_dir().to_string_lossy(),
            term_size(80, 24),
            Mode::Live { tmux: None },
        )
        .unwrap();
        let deadline = Instant::now() + std::time::Duration::from_secs(20);
        loop {
            let buffer = ptys
                .0
                .lock()
                .unwrap()
                .get("s")
                .and_then(|s| s.live.as_ref())
                .map(|l| l.lock().unwrap().buffer.clone())
                .unwrap_or_default();
            if buffer.contains("hello") {
                break;
            }
            assert!(Instant::now() < deadline, "output never buffered");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        ptys.kill("s");
        assert!(!ptys.has_live("s"));
    }

    #[test]
    fn prompt_heuristic_reads_the_last_visible_line() {
        let buf = "\u{1b}[2J\u{1b}[1;32mbuilding...\u{1b}[0m\nDo you want to proceed?\n";
        assert_eq!(last_line(buf), "Do you want to proceed?");
        assert!(looks_like_prompt(&last_line(buf)));
        assert!(looks_like_prompt("❯ 1. Yes"));
        assert!(looks_like_prompt("Continue [y/N]"));
        assert!(!looks_like_prompt("compiling recalc v0.1.0"));
        assert!(!looks_like_prompt(""));
    }

    #[test]
    fn strip_ansi_drops_csi_and_osc() {
        assert_eq!(strip_ansi("\u{1b}[31mred\u{1b}[0m"), "red");
        assert_eq!(strip_ansi("\u{1b}]0;title\u{7}text"), "text");
        assert_eq!(strip_ansi("\u{1b}]0;title\u{1b}\\text"), "text");
        assert_eq!(strip_ansi("a\r\nb"), "a\n\nb");
        // A spinner rewriting its line: the last visible text still wins.
        assert_eq!(
            last_line("working\rDo you want to proceed?"),
            "Do you want to proceed?"
        );
    }
}
