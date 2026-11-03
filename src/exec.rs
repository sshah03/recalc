//! Runs a resolved command under `sh -c` with a timeout, a process-group
//! kill, and capped capture of stdout and stderr.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio::sync::oneshot;

/// stderr keeps its last 8 KB, where an error's cause usually is.
pub(crate) const STDERR_CAP: usize = 8 * 1024;

/// Past this, stdout keeps its first and last halves and notes the cut. The
/// rest is still drained so the command never blocks on a full pipe.
pub const STDOUT_CAP: usize = 4 * 1024 * 1024;

/// How long to keep reading after the shell exits. A background process left
/// behind can hold the pipes open forever; it keeps running, output dropped.
pub(crate) const PIPE_GRACE: Duration = Duration::from_millis(300);

/// How long a stopped or timed-out run gets after SIGTERM before SIGKILL.
const TERM_GRACE: Duration = Duration::from_secs(2);

/// Process groups of one engine's runs in flight, killed when the engine
/// goes. Once killed it stays closed: a later spawn is killed at once.
#[derive(Clone)]
pub struct Groups(Shared);

/// The groups in flight, and whether the set is closed.
type Inner = Mutex<(HashSet<i32>, bool)>;
type Shared = Arc<Inner>;

/// Every engine's groups in this process, so a signal can end their runs
/// (`exit_on_signal`).
static LIVE: Mutex<Vec<std::sync::Weak<Inner>>> = Mutex::new(Vec::new());

impl Default for Groups {
    fn default() -> Self {
        let g: Shared = Arc::default();
        let mut live = LIVE.lock().expect("live groups lock");
        live.retain(|w| w.strong_count() > 0);
        live.push(Arc::downgrade(&g));
        Groups(g)
    }
}

/// Wait for SIGTERM, SIGINT or SIGHUP, end every run this process started,
/// and exit. Runs live in their own process groups, so without this a
/// killed window or a Ctrl-C'd command leaves its agents running and
/// spending.
pub async fn exit_on_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let (Ok(mut term), Ok(mut int), Ok(mut hup)) = (
        signal(SignalKind::terminate()),
        signal(SignalKind::interrupt()),
        signal(SignalKind::hangup()),
    ) else {
        return;
    };
    let code = tokio::select! {
        _ = term.recv() => libc::SIGTERM,
        _ = int.recv() => libc::SIGINT,
        _ = hup.recv() => libc::SIGHUP,
    };
    end_every_run();
    std::process::exit(128 + code);
}

/// Kill the runs of every engine in this process.
pub fn end_every_run() {
    let live: Vec<Shared> = LIVE
        .lock()
        .expect("live groups lock")
        .iter()
        .filter_map(std::sync::Weak::upgrade)
        .collect();
    for g in live {
        Groups(g).kill_all();
    }
}

impl Groups {
    /// SIGTERM every group, wait up to a second, then SIGKILL the rest.
    /// Blocking, since it runs on process exit.
    pub fn kill_all(&self) {
        let pgids: Vec<i32> = {
            let mut g = self.0.lock().expect("groups lock");
            g.1 = true;
            g.0.drain().collect()
        };
        for &pgid in &pgids {
            unsafe { libc::kill(-pgid, libc::SIGTERM) };
        }
        let deadline = Instant::now() + Duration::from_secs(1);
        while pgids.iter().any(|&g| group_alive(g)) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        for &pgid in pgids.iter().filter(|&&g| group_alive(g)) {
            unsafe { libc::kill(-pgid, libc::SIGKILL) };
        }
    }

    /// Enter a freshly spawned group; false (and killed) if already closed.
    fn join(&self, pgid: i32) -> bool {
        let mut g = self.0.lock().expect("groups lock");
        if g.1 {
            unsafe { libc::kill(-pgid, libc::SIGKILL) };
            return false;
        }
        g.0.insert(pgid);
        true
    }

    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.0.lock().unwrap().0.is_empty()
    }
}

/// Holds a group's place in `Groups` while its leader is unreaped.
pub(crate) struct Member<'a>(&'a Groups, i32);

impl Drop for Member<'_> {
    fn drop(&mut self) {
        self.0 .0.lock().expect("groups lock").0.remove(&self.1);
    }
}

#[derive(Debug)]
pub struct Output {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub duration_ms: u64,
}

/// What ended a run early.
pub const STOPPED: &str = "stopped";

/// Run `command` via `<shell> -c`, with `env` added to the inherited
/// environment. Err means launch failure, timeout or `cancel`; a non-zero
/// exit is a normal `Output`.
pub async fn run(
    shell: &str,
    command: &str,
    cwd: &Path,
    env: &HashMap<String, String>,
    timeout: Duration,
    cancel: oneshot::Receiver<()>,
    groups: &Groups,
) -> Result<Output, String> {
    let start = Instant::now();
    let Started {
        mut child,
        pgid,
        member: _member,
    } = spawn(shell, command, cwd, env, groups, false).await?;
    // Readers run on their own tasks so a pipe held open by a background
    // process never blocks the result.
    let out = Capture::spawn(
        child.stdout.take().expect("stdout is piped"),
        STDOUT_CAP / 2,
        STDOUT_CAP / 2,
    );
    let err = Capture::spawn(child.stderr.take().expect("stderr is piped"), 0, STDERR_CAP);
    // A dropped sender means the run can't be stopped, not that it was.
    let stopped = async {
        if cancel.await.is_err() {
            std::future::pending::<()>().await;
        }
    };
    let status = tokio::select! {
        status = tokio::time::timeout(timeout, child.wait()) => status,
        _ = stopped => {
            end_group(&mut child, pgid).await;
            return Err(STOPPED.into());
        }
    };
    let status = match status {
        Ok(Ok(status)) => status,
        Ok(Err(e)) => return Err(format!("failed to wait for the command: {e}")),
        Err(_) => {
            end_group(&mut child, pgid).await;
            return Err("timeout".into());
        }
    };
    let _ = tokio::time::timeout(PIPE_GRACE, async {
        let _ = tokio::join!(out.done(), err.done());
    })
    .await;
    Ok(Output {
        stdout: out.text("the middle"),
        stderr: err.text("the start"),
        // None only when killed by a signal.
        exit_code: status.code().unwrap_or(-1),
        duration_ms: start.elapsed().as_millis() as u64,
    })
}

/// A command in its own process group, registered in `Groups` until dropped.
pub(crate) struct Started<'a> {
    pub child: tokio::process::Child,
    pub pgid: Option<i32>,
    pub member: Option<Member<'a>>,
}

/// Start `command` under `<shell> -c` in a new process group, with stdin
/// piped only when `stdin` is set.
pub(crate) async fn spawn<'a>(
    shell: &str,
    command: &str,
    cwd: &Path,
    env: &HashMap<String, String>,
    groups: &'a Groups,
    stdin: bool,
) -> Result<Started<'a>, String> {
    let mut cmd = Command::new(shell);
    cmd.arg("-c")
        .arg(command)
        .current_dir(cwd)
        // No Python bytecode caches: a cache is trusted when size and
        // whole-second mtime match, so a same-length edit re-run within the
        // second would run stale code. The empty prefix dir skips existing caches.
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env(
            "PYTHONPYCACHEPREFIX",
            std::env::temp_dir().join("recalc-no-pycache"),
        )
        // Runs belong to the board, not to the agent that started this
        // engine (see `cli::me`).
        .env_remove("RECALC_AGENT")
        .envs(env)
        .stdin(if stdin { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = cmd.spawn().map_err(|e| format!("failed to launch: {e}"))?;
    let pgid = child.id().map(|p| p as i32);
    let member = match pgid {
        Some(g) if !groups.join(g) => {
            let _ = child.wait().await;
            return Err(STOPPED.into());
        }
        Some(g) => Some(Member(groups, g)),
        None => None,
    };
    Ok(Started {
        child,
        pgid,
        member,
    })
}

/// Whether any process remains in group `pgid`.
fn group_alive(pgid: i32) -> bool {
    unsafe { libc::kill(-pgid, 0) == 0 }
}

/// Stop a run's whole group: SIGTERM, wait up to `TERM_GRACE`, then SIGKILL.
pub(crate) async fn end_group(child: &mut tokio::process::Child, pgid: Option<i32>) {
    let Some(pgid) = pgid else {
        let _ = child.kill().await;
        return;
    };
    unsafe { libc::kill(-pgid, libc::SIGTERM) };
    let _ = tokio::time::timeout(TERM_GRACE, child.wait()).await;
    if group_alive(pgid) {
        unsafe { libc::kill(-pgid, libc::SIGKILL) };
    }
    let _ = child.wait().await;
}

/// One output stream read to EOF on its own task, keeping the first `head`
/// and last `tail` bytes and counting the rest.
pub(crate) struct Capture {
    kept: Arc<Mutex<Kept>>,
    done: tokio::task::JoinHandle<()>,
}

#[derive(Default)]
struct Kept {
    head: Vec<u8>,
    tail: std::collections::VecDeque<u8>,
    cut: usize,
}

impl Capture {
    pub(crate) fn spawn<R: AsyncRead + Unpin + Send + 'static>(
        mut r: R,
        head: usize,
        tail: usize,
    ) -> Capture {
        let kept = Arc::new(Mutex::new(Kept::default()));
        let k = kept.clone();
        let done = tokio::spawn(async move {
            let mut buf = [0u8; 64 * 1024];
            while let Ok(n) = r.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                let mut k = k.lock().expect("capture lock");
                let room = head.saturating_sub(k.head.len());
                let (to_head, rest) = buf[..n].split_at(n.min(room));
                k.head.extend_from_slice(to_head);
                k.tail.extend(rest);
                let over = k.tail.len().saturating_sub(tail);
                k.tail.drain(..over);
                k.cut += over;
            }
        });
        Capture { kept, done }
    }

    /// Wait for EOF. The task keeps draining if the caller stops waiting.
    pub(crate) async fn done(&self) {
        while !self.done.is_finished() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// The kept text, with a note naming `at` where bytes were cut.
    pub(crate) fn text(&self, at: &str) -> String {
        let k = self.kept.lock().expect("capture lock");
        let mut bytes = k.head.clone();
        let note = format!(
            "[recalc: {} cut from {at}; pipe the output to a file and read that instead]",
            size(k.cut)
        );
        match (k.cut > 0, k.head.is_empty()) {
            (false, _) => {}
            (true, true) => {
                bytes.extend_from_slice(note.as_bytes());
                bytes.push(b'\n');
            }
            (true, false) => {
                bytes.extend_from_slice(format!("\n{note}\n").as_bytes());
            }
        }
        bytes.extend(k.tail.iter());
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

fn size(bytes: usize) -> String {
    match bytes {
        b if b >= 1024 * 1024 => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
        b if b >= 1024 => format!("{} KB", b / 1024),
        b => format!("{b} bytes"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cwd() -> std::path::PathBuf {
        std::env::temp_dir()
    }

    fn never() -> oneshot::Receiver<()> {
        oneshot::channel().1
    }

    #[tokio::test]
    async fn captures_everything() {
        let out = run(
            "sh",
            "echo out; echo err >&2; exit 3",
            &cwd(),
            &Default::default(),
            Duration::from_secs(30),
            never(),
            &Groups::default(),
        )
        .await
        .unwrap();
        assert_eq!(out.stdout, "out\n");
        assert_eq!(out.stderr, "err\n");
        assert_eq!(out.exit_code, 3);
    }

    #[tokio::test]
    async fn timeout_kills_the_process_group() {
        let start = Instant::now();
        let err = run(
            "sh",
            "sleep 30",
            &cwd(),
            &Default::default(),
            Duration::from_millis(100),
            never(),
            &Groups::default(),
        )
        .await
        .unwrap_err();
        assert_eq!(err, "timeout");
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn cancel_kills_the_run_promptly() {
        let (tx, rx) = oneshot::channel();
        let start = Instant::now();
        let handle = tokio::spawn(async move {
            run(
                "sh",
                "sleep 30",
                &std::env::temp_dir(),
                &Default::default(),
                Duration::from_secs(60),
                rx,
                &Groups::default(),
            )
            .await
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        tx.send(()).unwrap();
        assert_eq!(handle.await.unwrap().unwrap_err(), STOPPED);
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn stdout_is_capped_and_the_run_still_finishes() {
        let out = run(
            "sh",
            &format!("head -c {} /dev/zero; echo done >&2", STDOUT_CAP * 2),
            &cwd(),
            &Default::default(),
            Duration::from_secs(30),
            never(),
            &Groups::default(),
        )
        .await
        .unwrap();
        assert!(out.stdout.len() < STDOUT_CAP + 200);
        assert!(out.stdout.contains("MB cut from the middle"));
        assert_eq!(out.stderr, "done\n");
        assert_eq!(out.exit_code, 0);
    }

    #[tokio::test]
    async fn kill_all_takes_down_runs_in_flight() {
        let groups = Groups::default();
        let g = groups.clone();
        let start = Instant::now();
        let handle = tokio::spawn(async move {
            run(
                "sh",
                "sleep 30",
                &std::env::temp_dir(),
                &Default::default(),
                Duration::from_secs(60),
                never(),
                &g,
            )
            .await
        });
        while groups.is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        groups.kill_all();
        let out = handle.await.unwrap().unwrap();
        assert_eq!(out.exit_code, -1, "killed by a signal");
        assert!(start.elapsed() < Duration::from_secs(5));
        assert!(groups.is_empty());
    }

    async fn sh(command: &str, timeout: Duration) -> Result<Output, String> {
        run(
            "sh",
            command,
            &cwd(),
            &Default::default(),
            timeout,
            never(),
            &Groups::default(),
        )
        .await
    }

    #[tokio::test]
    async fn a_process_left_in_the_background_doesnt_hold_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let pid = dir.path().join("pid");
        let start = Instant::now();
        let out = sh(
            &format!("(sleep 20 & echo $! > {}); echo hi", pid.display()),
            Duration::from_secs(10),
        )
        .await
        .expect("the shell finished; the run did too");
        assert_eq!(out.stdout, "hi\n");
        assert_eq!(out.exit_code, 0);
        assert!(start.elapsed() < Duration::from_secs(3));
        let pid: i32 = std::fs::read_to_string(&pid)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        unsafe { libc::kill(pid, libc::SIGKILL) };
    }

    #[tokio::test]
    async fn long_output_keeps_its_end() {
        let n = STDOUT_CAP; // lines of at least two bytes: twice the cap
        let out = sh(&format!("seq 1 {n}; echo SUMMARY"), Duration::from_secs(30))
            .await
            .unwrap();
        assert!(out.stdout.starts_with("1\n2\n3\n"));
        assert!(out.stdout.ends_with("SUMMARY\n"));
        assert!(out.stdout.contains("cut from the middle"));
        let err = sh("seq 1 100000 >&2; echo LAST >&2", Duration::from_secs(30))
            .await
            .unwrap();
        assert!(err.stderr.ends_with("LAST\n"));
        assert!(err.stderr.starts_with("[recalc: "));
    }

    #[tokio::test]
    async fn a_stopped_run_gets_sigterm_first() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("cleaned-up");
        let (tx, rx) = oneshot::channel();
        let command = format!(
            "trap 'echo bye > {}; exit 0' TERM; sleep 30 & wait",
            marker.display()
        );
        let handle = tokio::spawn(async move {
            run(
                "sh",
                &command,
                &std::env::temp_dir(),
                &Default::default(),
                Duration::from_secs(60),
                rx,
                &Groups::default(),
            )
            .await
        });
        tokio::time::sleep(Duration::from_millis(200)).await;
        tx.send(()).unwrap();
        assert_eq!(handle.await.unwrap().unwrap_err(), STOPPED);
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "bye\n");
    }

    #[tokio::test]
    async fn python_runs_without_bytecode_caches() {
        let out = sh(
            "echo $PYTHONDONTWRITEBYTECODE; basename \"$PYTHONPYCACHEPREFIX\"",
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        assert_eq!(out.stdout.trim(), "1\nrecalc-no-pycache");
    }

    #[tokio::test]
    async fn launch_failure_is_error() {
        assert!(run(
            "sh",
            "true",
            Path::new("/nonexistent-dir"),
            &Default::default(),
            Duration::from_secs(30),
            never(),
            &Groups::default(),
        )
        .await
        .is_err());
    }
}
