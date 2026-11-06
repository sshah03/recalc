//! tmux-backed agent sessions that outlive their window.
//!
//! Each `@` cell's session is a tmux session on a recalc-owned server
//! (`-L recalc`), one per cell per board; the cell's terminal and any desktop
//! terminal are clients of it. Closing the window only detaches. Without tmux
//! on PATH a session is a bare PTY that dies with the app.

use crate::interp::shell_quote;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The tmux socket name. A separate server keeps the user's `tmux ls` clean
/// and lets recalc use its own config.
pub const SOCKET: &str = "recalc";

/// The driver's session name; the leading underscore avoids clashing with cell names.
pub const DRIVER: &str = "_driver";

/// Used instead of the user's `.tmux.conf`. `window-size latest` keeps a wide
/// desktop terminal from being squeezed to the cell's width.
const CONF: &str = "\
set -g status off
set -g mouse on
set -g history-limit 50000
set -g default-terminal \"xterm-256color\"
set -sg escape-time 10
set -g focus-events on
set -g window-size latest
set -g set-titles off
set -g exit-empty on
";

pub fn available() -> bool {
    crate::shell_env::which("tmux").is_some()
}

/// `recalc-<board>-<cell>`, where `<board>` is six hex digits of the board's
/// path hash; anything but letters, digits, `_` and `-` in the cell name
/// becomes `_`, with a short hash so two names can't collide
/// (`Notes!B2` → `Notes_B2-1a2b`).
pub fn name(dir: &Path, cell: &str) -> String {
    let path = dir
        .canonicalize()
        .unwrap_or_else(|_| dir.to_path_buf())
        .to_string_lossy()
        .into_owned();
    // FNV-1a: stable across runs and machines, which a `HashMap` hasher isn't.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in path.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    // Only plain characters: tmux reserves `.` and `:`, and a sheet's `!`,
    // quotes and spaces would need quoting in every command that names it.
    let flat: String = cell
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    // Two names can flatten alike (`Notes!B2`, `Notes_B2`); a changed one
    // carries a hash of what it was. Names tmux takes as they are don't.
    let flat = if flat == cell {
        flat
    } else {
        format!("{flat}-{:04x}", fnv(cell) & 0xffff)
    };
    format!("recalc-{:06x}-{flat}", h & 0xff_ffff)
}

/// Under `$HOME`, not /tmp: another user could plant a conf at a fixed /tmp
/// path and tmux would run it.
fn conf_path() -> PathBuf {
    match std::env::var_os("HOME") {
        Some(home) => PathBuf::from(home).join(".cache/recalc/tmux.conf"),
        None => {
            std::env::temp_dir().join(format!("recalc-{}-tmux.conf", unsafe { libc::getuid() }))
        }
    }
}

fn ensure_conf() -> PathBuf {
    let path = conf_path();
    if std::fs::read_to_string(&path).ok().as_deref() != Some(CONF) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // Temp then rename, so a starting tmux never reads a partial file.
        let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
        if std::fs::write(&tmp, CONF).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
    path
}

fn tmux() -> Command {
    let mut c = Command::new("tmux");
    c.arg("-L").arg(SOCKET);
    c.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    c
}

/// The shell command that runs `cmd` in tmux session `name` in `cwd`, or
/// attaches if it already exists (`-A`). Writes the conf first.
pub fn wrap(cmd: &str, name: &str, cwd: &Path) -> String {
    let conf = ensure_conf();
    // A session takes the tmux server's environment, which may predate this
    // app's; the agent must look for its conversations where recalc looked.
    let homes: String = AGENT_HOMES
        .iter()
        .filter_map(|k| {
            std::env::var(k)
                .ok()
                .map(|v| format!("export {k}={}; ", shell_quote(&v)))
        })
        .collect();
    let cmd = format!("{homes}{cmd}");
    format!(
        "tmux -L {SOCKET} -f {} new-session -A -s {} -c {} {}",
        shell_quote(&conf.to_string_lossy()),
        shell_quote(name),
        shell_quote(&cwd.to_string_lossy()),
        shell_quote(&cmd),
    )
}

/// Whether the tmux session exists. `=` matches the exact name, not a prefix.
pub fn alive(name: &str) -> bool {
    tmux()
        .arg("has-session")
        .arg("-t")
        .arg(format!("={name}"))
        .status()
        .is_ok_and(|s| s.success())
}

/// End the tmux session and the agent in it.
pub fn kill(name: &str) {
    let _ = tmux()
        .arg("kill-session")
        .arg("-t")
        .arg(format!("={name}"))
        .status();
}

/// The shell command that attaches a terminal to the session.
pub fn attach_command(name: &str) -> String {
    format!(
        "tmux -L {SOCKET} attach-session -t {}",
        shell_quote(&format!("={name}"))
    )
}

/// Open a desktop terminal running `cmd`. On macOS, Terminal runs it through
/// a profile (see `open_in_terminal_app`) and another app opens a `.command`
/// file; neither needs automation permission. On Linux it tries common
/// emulators, `$TERMINAL` first.
pub fn open_in_terminal(cmd: &str, app: Option<&str>) -> Result<(), String> {
    if cfg!(target_os = "macos") && app.is_none_or(|a| a == "Terminal") {
        if let Some(result) = open_in_terminal_app(cmd) {
            return result;
        }
    }
    if cfg!(target_os = "macos") {
        let file = std::env::temp_dir().join(format!("recalc-{:x}.command", fnv(cmd)));
        // Not `exec`: a resume is `cd … && export …; claude …`, and `exec cd`
        // would replace the shell with /usr/bin/cd and run nothing after it.
        // This process's PATH: an app like iTerm runs the file directly, with
        // launchd's PATH, where tmux and claude aren't.
        let path = std::env::var("PATH").unwrap_or_default();
        std::fs::write(
            &file,
            format!("#!/bin/sh\nexport PATH={}\n{cmd}\n", shell_quote(&path)),
        )
        .map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755))
                .map_err(|e| e.to_string())?;
        }
        // WezTerm drops a file it's handed while it starts up; its own launcher
        // runs the command whether or not it's already open.
        if let Some(wezterm) = app
            .filter(|a| a.eq_ignore_ascii_case("wezterm"))
            .and_then(|_| wezterm_cli())
        {
            return Command::new(wezterm)
                .args(["start", "--", "/bin/sh"])
                .arg(&file)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map(|_| ())
                .map_err(|e| {
                    format!("could not start WezTerm ({e}); run this in a terminal instead:\n{cmd}")
                });
        }
        let status = Command::new("open")
            .arg("-a")
            .arg(app.unwrap_or("Terminal"))
            .arg(&file)
            .status()
            .map_err(|e| format!("open: {e}"))?;
        return if status.success() {
            Ok(())
        } else {
            Err(format!(
                "could not open {}; run this in a terminal instead:\n{cmd}",
                app.unwrap_or("Terminal")
            ))
        };
    }
    let env_term = std::env::var("TERMINAL").ok();
    let mut candidates: Vec<(String, Vec<&str>)> = Vec::new();
    if let Some(a) = app {
        candidates.push((a.to_string(), vec!["-e"]));
    }
    if let Some(t) = env_term {
        candidates.push((t, vec!["-e"]));
    }
    for (bin, args) in [
        ("x-terminal-emulator", vec!["-e"]),
        ("gnome-terminal", vec!["--"]),
        ("konsole", vec!["-e"]),
        ("xfce4-terminal", vec!["-e"]),
        ("alacritty", vec!["-e"]),
        ("kitty", vec![]),
        ("xterm", vec!["-e"]),
    ] {
        candidates.push((bin.to_string(), args));
    }
    for (bin, args) in candidates {
        if crate::shell_env::which(&bin).is_none() {
            continue;
        }
        let spawned = Command::new(&bin)
            .args(&args)
            .arg("sh")
            .arg("-c")
            .arg(cmd)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        if spawned.is_ok() {
            return Ok(());
        }
    }
    Err(format!(
        "no terminal emulator found; run this in a terminal instead:\n{cmd}"
    ))
}

/// WezTerm's command-line launcher: on PATH, or inside the app.
fn wezterm_cli() -> Option<PathBuf> {
    crate::shell_env::which("wezterm").or_else(|| {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        [PathBuf::from("/Applications"), home.join("Applications")]
            .into_iter()
            .map(|d| d.join("WezTerm.app/Contents/MacOS/wezterm"))
            .find(|p| p.is_file())
    })
}

/// Terminal opens a `.command` file by typing its path into a new login
/// shell, so anything in the shell's startup that reads a key (oh-my-zsh's
/// update prompt) eats part of it. Instead the command is queued, and Terminal
/// opens `recalc.terminal`: a profile that runs a fixed runner in place of a
/// shell, which takes the oldest queued command. The profile never changes, so
/// Terminal imports it once, as "recalc". Terminal splits the profile's command
/// on spaces, so a temp dir with a space in it falls back to the `.command`.
fn open_in_terminal_app(cmd: &str) -> Option<Result<(), String>> {
    let dir = std::env::temp_dir().join("recalc-terminal");
    if dir.to_string_lossy().contains(char::is_whitespace) {
        return None;
    }
    Some(open_via_profile(&dir, cmd).map_err(|e| {
        format!("could not open Terminal ({e}); run this in a terminal instead:\n{cmd}")
    }))
}

fn open_via_profile(dir: &Path, cmd: &str) -> std::io::Result<()> {
    let runner = queue_command(dir, cmd)?;
    let profile = dir.join("recalc.terminal");
    std::fs::write(&profile, terminal_profile(&runner.to_string_lossy()))?;
    let status = Command::new("open")
        .args(["-a", "Terminal"])
        .arg(&profile)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other("open exited with an error"))
    }
}

/// Queue `cmd`, with this process's PATH, under `dir/queue`, and write the
/// runner that claims the oldest queued command by renaming it. Returns the
/// runner's path.
fn queue_command(dir: &Path, cmd: &str) -> std::io::Result<PathBuf> {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let queue = dir.join("queue");
    std::fs::create_dir_all(&queue)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 10_000;
    let path = std::env::var("PATH").unwrap_or_default();
    let job = format!(
        // The command as written, not `exec`ed: it may be compound (see above).
        "rm -f -- \"$0\"\nexport PATH={}\n{cmd}\n",
        shell_quote(&path)
    );
    let name = format!("{stamp:020}-{seq:04}");
    let partial = queue.join(format!("{name}.part"));
    std::fs::write(&partial, job)?;
    std::fs::rename(&partial, queue.join(format!("{name}.sh")))?;
    let runner = dir.join("run");
    std::fs::write(
        &runner,
        format!(
            "#!/bin/sh\nfor f in {}/*.sh; do\n  mv \"$f\" \"$f.run\" 2>/dev/null && exec /bin/sh \"$f.run\"\ndone\necho \"recalc: nothing to open\"\n",
            shell_quote(&queue.to_string_lossy())
        ),
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&runner, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(runner)
}

/// A Terminal profile whose window runs `command` instead of a shell and
/// closes when it exits cleanly.
fn terminal_profile(command: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>name</key><string>recalc</string>
	<key>type</key><string>Window Settings</string>
	<key>ProfileCurrentVersion</key><real>2.07</real>
	<key>CommandString</key><string>{command}</string>
	<key>RunCommandAsShell</key><true/>
	<key>shellExitAction</key><integer>1</integer>
</dict>
</plist>
"#
    )
}

fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// Whether Codex kept a conversation `id` (a `rollout-…-<id>.jsonl` under
/// `$CODEX_HOME/sessions`), so `codex resume <id>` has something to resume.
pub fn codex_session_exists(id: &str) -> bool {
    let Some(home) = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".codex")))
    else {
        return false;
    };
    fn walk(dir: &Path, id: &str, depth: u32) -> bool {
        std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .any(|e| {
                let p = e.path();
                if depth > 0 && p.is_dir() {
                    walk(&p, id, depth - 1)
                } else {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.ends_with(&format!("-{id}.jsonl")))
                }
            })
    }
    !id.is_empty() && walk(&home.join("sessions"), id, 3)
}

/// Where agents keep their conversations, when set elsewhere than the default.
const AGENT_HOMES: [&str; 2] = ["CLAUDE_CONFIG_DIR", "CODEX_HOME"];

/// Whether Claude Code kept a conversation `id` on this computer (a
/// `<id>.jsonl` in one of `$CLAUDE_CONFIG_DIR/projects/*`). It finds one by id
/// wherever the board's folder has moved since, but not one from another
/// computer: `claude --resume` then stops with "No conversation found".
pub fn claude_session_exists(id: &str) -> bool {
    let Some(home) = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".claude")))
    else {
        return false;
    };
    claude_session_in(&home, id)
}

fn claude_session_in(home: &Path, id: &str) -> bool {
    let file = format!("{id}.jsonl");
    !id.is_empty()
        && std::fs::read_dir(home.join("projects"))
            .into_iter()
            .flatten()
            .flatten()
            .any(|project| project.path().join(&file).is_file())
}

/// Whether pi has conversation `id` here. pi keeps one file per session,
/// `<sessions>/<folder>/<time>_<id>.jsonl`, under `PI_CODING_AGENT_SESSION_DIR`
/// or the agent directory (`PI_CODING_AGENT_DIR`, else `~/.pi/agent`).
pub fn pi_session_exists(id: &str) -> bool {
    let sessions = std::env::var_os("PI_CODING_AGENT_SESSION_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("PI_CODING_AGENT_DIR").map(|d| PathBuf::from(d).join("sessions"))
        })
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".pi/agent/sessions")));
    sessions.is_some_and(|dir| pi_session_in(&dir, id))
}

fn pi_session_in(sessions: &Path, id: &str) -> bool {
    let suffix = format!("_{id}.jsonl");
    !id.is_empty()
        && std::fs::read_dir(sessions)
            .into_iter()
            .flatten()
            .flatten()
            .flat_map(|folder| {
                std::fs::read_dir(folder.path())
                    .into_iter()
                    .flatten()
                    .flatten()
            })
            .any(|f| f.file_name().to_string_lossy().ends_with(&suffix))
}

/// What one Claude Code session used, as Claude Code itself recorded it when
/// it ended: the last session that ran in a folder.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LastRun {
    pub session: String,
    /// When that process started, in Unix milliseconds: a resumed
    /// conversation's later runs have the same session and a new start.
    pub started: u64,
    /// Claude Code's own figure, at API prices.
    pub cost_usd: f64,
    /// Every token, cached context included, as a cell's run counts them.
    pub tokens: u64,
    /// The model that did most of it.
    pub model: Option<String>,
}

/// Claude Code's state file, where it records each folder's last session:
/// `$CLAUDE_CONFIG_DIR/.claude.json`, else `~/.claude.json`.
pub fn claude_state_path() -> Option<PathBuf> {
    match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(dir) => Some(PathBuf::from(dir).join(".claude.json")),
        None => std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".claude.json")),
    }
}

/// The last Claude Code session that ran in `dir`, from its state file.
pub fn claude_last_run(state: &Path, dir: &Path) -> Option<LastRun> {
    let text = std::fs::read_to_string(state).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    let want = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let (_, p) = json["projects"].as_object()?.iter().find(|(k, _)| {
        let k = Path::new(k.as_str());
        k == want || k.canonicalize().is_ok_and(|k| k == want)
    })?;
    let n = |k: &str| p[k].as_u64().unwrap_or(0);
    let model = p["lastModelUsage"].as_object().and_then(|m| {
        m.iter()
            .max_by(|a, b| {
                let c = |v: &serde_json::Value| v["costUSD"].as_f64().unwrap_or(0.0);
                c(a.1).total_cmp(&c(b.1))
            })
            .map(|(k, _)| k.clone())
    });
    Some(LastRun {
        session: p["lastSessionId"].as_str()?.to_string(),
        started: p["lastStartTime"].as_u64()?,
        cost_usd: p["lastCost"].as_f64()?,
        tokens: n("lastTotalInputTokens")
            + n("lastTotalOutputTokens")
            + n("lastTotalCacheCreationInputTokens")
            + n("lastTotalCacheReadInputTokens"),
        model,
    })
}

/// Whether `agent` can resume conversation `id` here. Agents recalc can't look
/// into (a custom `[agents]` command) are taken at their word.
pub fn conversation_exists(agent: &str, id: &str) -> bool {
    match agent {
        "claude" => claude_session_exists(id),
        "codex" => codex_session_exists(id),
        "pi" => pi_session_exists(id),
        _ => true,
    }
}

/// Seconds since the epoch of a UTC timestamp like `2026-10-07T06:22:28.351Z`.
fn utc_seconds(ts: &str) -> Option<f64> {
    let (date, time) = ts.strip_suffix('Z')?.split_once('T')?;
    let mut d = date.splitn(3, '-').map(|x| x.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let mut t = time.splitn(3, ':');
    let (h, min) = (
        t.next()?.parse::<i64>().ok()?,
        t.next()?.parse::<i64>().ok()?,
    );
    let sec = t.next()?.parse::<f64>().ok()?;
    // Days from 1970-01-01 to y-m-day (Howard Hinnant's days_from_civil).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some((days * 86_400 + h * 3_600 + min * 60) as f64 + sec)
}

/// The id of the interactive Codex session started in `cwd` since `since`,
/// other than the ones in `taken` (conversations the board already holds).
/// Codex can't be given an id at launch, so this reads the `session_meta`
/// first line of `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-*.jsonl`. Headless
/// (`codex_exec`) runs are skipped; they take their id from their output.
/// It goes by when a conversation started, not when its file last changed:
/// a cell's conversation started a moment earlier is still being written to.
pub fn find_codex_session(
    cwd: &Path,
    since: std::time::SystemTime,
    taken: &[String],
) -> Option<String> {
    let home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".codex")))?;
    codex_session_in(&home, cwd, since, taken)
}

fn codex_session_in(
    home: &Path,
    cwd: &Path,
    since: std::time::SystemTime,
    taken: &[String],
) -> Option<String> {
    let want = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    // Newest two per level: a session started before midnight is filed under the previous day.
    let newest = |dir: &Path| -> Vec<PathBuf> {
        let mut d: Vec<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .collect();
        d.sort();
        d.into_iter().rev().take(2).collect()
    };
    let mut found: Option<(std::time::SystemTime, String)> = None;
    for year in newest(&home.join("sessions")) {
        for month in newest(&year) {
            for day in newest(&month) {
                for file in std::fs::read_dir(&day).into_iter().flatten().flatten() {
                    let path = file.path();
                    let Ok(modified) = file.metadata().and_then(|m| m.modified()) else {
                        continue;
                    };
                    if modified < since || path.extension().is_none_or(|e| e != "jsonl") {
                        continue;
                    }
                    let Some(meta) = std::fs::File::open(&path).ok().and_then(|f| {
                        let mut line = String::new();
                        std::io::BufRead::read_line(&mut std::io::BufReader::new(f), &mut line)
                            .ok()?;
                        serde_json::from_str::<serde_json::Value>(&line).ok()
                    }) else {
                        continue;
                    };
                    let p = &meta["payload"];
                    let started = p["timestamp"]
                        .as_str()
                        .and_then(utc_seconds)
                        .map(|s| {
                            std::time::UNIX_EPOCH + std::time::Duration::from_secs_f64(s.max(0.0))
                        })
                        .unwrap_or(modified);
                    if started < since {
                        continue;
                    }
                    let dir = p["cwd"].as_str().map(PathBuf::from);
                    let dir = dir.map(|d| d.canonicalize().unwrap_or(d));
                    if meta["type"] != "session_meta"
                        || dir.as_deref() != Some(want.as_path())
                        || p["originator"] == "codex_exec"
                    {
                        continue;
                    }
                    // The earliest start is the one this launch made.
                    if let Some(id) = p["id"].as_str().filter(|id| !taken.iter().any(|t| t == id)) {
                        if found.as_ref().is_none_or(|(t, _)| started < *t) {
                            found = Some((started, id.to_string()));
                        }
                    }
                }
            }
        }
    }
    found.map(|(_, id)| id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_stable_per_board_and_legal_for_tmux() {
        let a = name(Path::new("/tmp/board-a"), "B2");
        let b = name(Path::new("/tmp/board-b"), "B2");
        assert_ne!(a, b, "two boards' B2 must not share a session");
        assert_eq!(a, name(Path::new("/tmp/board-a"), "B2"));
        assert!(a.starts_with("recalc-") && a.ends_with("-B2"), "{a}");
        assert_eq!(a.len(), "recalc-".len() + 6 + "-B2".len());
        assert!(name(Path::new("/x"), "a.b:c").contains("-a_b_c-"));
        assert!(name(Path::new("/x"), DRIVER).ends_with("-_driver"));
    }

    #[test]
    fn a_claude_conversation_is_found_by_id_in_any_project() {
        let home = tempfile::tempdir().unwrap();
        let project = home.path().join("projects/-Users-me-old-place");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("abc.jsonl"), "{}\n").unwrap();
        assert!(claude_session_in(home.path(), "abc"));
        assert!(!claude_session_in(home.path(), "from-another-computer"));
        assert!(!claude_session_in(home.path(), ""));
        assert!(!claude_session_in(&home.path().join("nowhere"), "abc"));
        assert!(
            conversation_exists("my-own-agent", "anything"),
            "agents recalc can't look into are taken at their word"
        );
    }

    #[test]
    fn a_pi_conversation_is_found_by_its_file_name() {
        let sessions = tempfile::tempdir().unwrap();
        let folder = sessions.path().join("--Users-me-board--");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("2026-11-05T19-28-18-005Z_01a1ad2e-f453.jsonl"),
            "{}",
        )
        .unwrap();
        assert!(pi_session_in(sessions.path(), "01a1ad2e-f453"));
        assert!(
            !pi_session_in(sessions.path(), "f453"),
            "the whole id, not its end"
        );
        assert!(!pi_session_in(sessions.path(), ""));
        assert!(!pi_session_in(
            &sessions.path().join("nowhere"),
            "01a1ad2e-f453"
        ));
    }

    #[test]
    fn sheet_cells_get_tmux_names_of_their_own() {
        let dir = Path::new("/repo");
        let (a, b) = (name(dir, "Notes!B2"), name(dir, "Notes_B2"));
        assert_ne!(a, b);
        assert!(a.contains("-Notes_B2-") && !a.contains('!'), "{a}");
        assert!(
            b.ends_with("-Notes_B2"),
            "a name tmux takes as it is stays: {b}"
        );
        assert_ne!(name(dir, "My Notes!B2"), name(dir, "My_Notes!B2"));
    }

    #[test]
    fn wrap_attaches_or_creates_and_quotes_the_command() {
        let w = wrap(
            "claude 'do it' --mcp-config '/b/m.json'",
            "recalc-abc123-B2",
            Path::new("/repo"),
        );
        assert!(w.starts_with("tmux -L recalc -f "), "{w}");
        assert!(
            w.contains(" new-session -A -s 'recalc-abc123-B2' -c '/repo' "),
            "{w}"
        );
        assert!(
            w.ends_with("'claude '\\''do it'\\'' --mcp-config '\\''/b/m.json'\\'''"),
            "{w}"
        );
        assert_eq!(std::fs::read_to_string(conf_path()).unwrap(), CONF);
    }

    #[test]
    fn queued_commands_run_once_each_oldest_first() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out");
        let o = shell_quote(&out.to_string_lossy());
        queue_command(dir.path(), &format!("sh -c 'echo first >> {o}'")).unwrap();
        let runner = queue_command(dir.path(), &format!("sh -c 'echo second >> {o}'")).unwrap();
        let run = || {
            let o = Command::new(&runner).output().unwrap();
            String::from_utf8_lossy(&o.stdout).into_owned()
        };
        run();
        run();
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "first\nsecond\n");
        assert!(run().contains("nothing to open"));
        // A resume is compound (`cd … && export …; claude …`); all of it runs.
        let elsewhere = dir.path().join("wd");
        std::fs::create_dir(&elsewhere).unwrap();
        let w = shell_quote(&elsewhere.to_string_lossy());
        queue_command(
            dir.path(),
            &format!("cd {w} && export RECALC_AGENT='B2'; echo \"$RECALC_AGENT in $(basename $PWD)\" >> {o}"),
        )
        .unwrap();
        run();
        assert!(std::fs::read_to_string(&out)
            .unwrap()
            .ends_with("B2 in wd\n"));
        assert_eq!(
            std::fs::read_dir(dir.path().join("queue")).unwrap().count(),
            0
        );
    }

    #[test]
    fn attach_command_names_the_exact_session() {
        assert_eq!(
            attach_command("recalc-abc123-B2"),
            "tmux -L recalc attach-session -t '=recalc-abc123-B2'"
        );
    }

    #[test]
    fn a_session_that_never_existed_is_not_alive() {
        // Whether or not tmux is installed, a random name has no session.
        assert!(!alive("recalc-000000-nonexistent-cell"));
    }

    #[test]
    fn a_codex_session_is_found_by_its_directory() {
        let codex = tempfile::tempdir().unwrap();
        let board = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let day = codex.path().join("sessions/2026/09/23");
        std::fs::create_dir_all(&day).unwrap();
        let since = std::time::SystemTime::now() - std::time::Duration::from_secs(1);
        let meta = |id: &str, cwd: &Path, originator: &str| {
            format!(
                "{}\n",
                serde_json::json!({"type": "session_meta", "payload": {"id": id, "cwd": cwd, "originator": originator}})
            )
        };
        std::fs::write(
            day.join("rollout-a.jsonl"),
            meta("elsewhere", other.path(), "codex_cli_rs"),
        )
        .unwrap();
        std::fs::write(
            day.join("rollout-b.jsonl"),
            meta("headless", board.path(), "codex_exec"),
        )
        .unwrap();
        std::fs::write(
            day.join("rollout-c.jsonl"),
            meta("mine", board.path(), "codex_cli_rs"),
        )
        .unwrap();
        let found = codex_session_in(codex.path(), board.path(), since, &[]);
        assert_eq!(found.as_deref(), Some("mine"));
    }

    #[test]
    fn a_codex_driver_is_not_given_the_conversation_a_cell_started_just_before() {
        let codex = tempfile::tempdir().unwrap();
        let board = tempfile::tempdir().unwrap();
        let day = codex.path().join("sessions/2026/10/06");
        std::fs::create_dir_all(&day).unwrap();
        let at = |secs: u64| {
            let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs);
            let s = t.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
            let (days, rest) = (s / 86_400, s % 86_400);
            // Back to a civil date (Hinnant's civil_from_days) for the timestamp.
            let z = days + 719_468;
            let era = z.div_euclid(146_097);
            let doe = z - era * 146_097;
            let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
            let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
            let mp = (5 * doy + 2) / 153;
            let d = doy - (153 * mp + 2) / 5 + 1;
            let m = if mp < 10 { mp + 3 } else { mp - 9 };
            let y = yoe + era * 400 + i64::from(m <= 2);
            format!(
                "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.500Z",
                rest / 3600,
                rest % 3600 / 60,
                rest % 60
            )
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let meta = |id: &str, started: u64| {
            format!(
                "{}\n",
                serde_json::json!({"type": "session_meta", "payload": {"id": id, "cwd": board.path(), "originator": "codex-tui", "timestamp": at(started)}})
            )
        };
        // B2's session started 4 s before the driver and is still being written.
        std::fs::write(day.join("rollout-b2.jsonl"), meta("cell", now - 4)).unwrap();
        std::fs::write(day.join("rollout-drv.jsonl"), meta("driver", now)).unwrap();
        let since = std::time::UNIX_EPOCH + std::time::Duration::from_secs(now - 2);
        let found = codex_session_in(codex.path(), board.path(), since, &[]);
        // Two started together: the one the board already holds isn't taken twice.
        let early = std::time::UNIX_EPOCH + std::time::Duration::from_secs(now - 10);
        let other = codex_session_in(codex.path(), board.path(), early, &["cell".to_string()]);
        assert_eq!(found.as_deref(), Some("driver"));
        assert_eq!(other.as_deref(), Some("driver"));
        assert_eq!(utc_seconds("1970-01-02T00:00:01.5Z"), Some(86_401.5));
        assert_eq!(
            utc_seconds("2026-10-07T06:22:28.351Z").map(|s| s as u64),
            Some(1_791_354_148)
        );
    }
}
