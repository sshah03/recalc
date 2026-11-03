//! PATH setup for an app launched from Finder or the Dock, which inherits
//! launchd's minimal PATH. At startup we adopt the login shell's PATH, as
//! VS Code and Zed do.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const MARK: &str = "__recalc_path__";

/// Prepend the login shell's PATH entries to this process's PATH. Silent on
/// failure: a broken shell config must not keep the app from opening.
pub fn adopt_login_shell_path() {
    let Some(shell_path) = login_shell_path() else {
        return;
    };
    let current = std::env::var_os("PATH").unwrap_or_default();
    let mut merged: Vec<PathBuf> = std::env::split_paths(&shell_path).collect();
    for p in std::env::split_paths(&current) {
        if !merged.contains(&p) {
            merged.push(p);
        }
    }
    if let Ok(joined) = std::env::join_paths(merged) {
        std::env::set_var("PATH", joined);
    }
}

/// Put the running binary's directory first on PATH so `recalc` works in
/// every shell the app starts; inside an app bundle it is on no PATH.
pub fn put_own_dir_on_path() {
    let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    else {
        return;
    };
    let current = std::env::var_os("PATH").unwrap_or_default();
    let mut paths: Vec<PathBuf> = std::env::split_paths(&current)
        .filter(|p| *p != dir)
        .collect();
    paths.insert(0, dir);
    if let Ok(joined) = std::env::join_paths(paths) {
        std::env::set_var("PATH", joined);
    }
}

/// The PATH of an interactive login shell, or None if it doesn't answer in time.
pub fn login_shell_path() -> Option<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    // Interactive + login so every rc file runs; the markers separate the
    // answer from whatever those files print.
    let script = format!("printf '{MARK}%s{MARK}' \"$PATH\"");
    let mut child = Command::new(&shell)
        .args(["-i", "-l", "-c", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut out = String::new();
        let _ = std::io::Read::read_to_string(&mut stdout, &mut out);
        let _ = tx.send(out);
    });
    let out = match rx.recv_timeout(Duration::from_secs(4)) {
        Ok(out) => out,
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
    };
    let _ = child.wait();
    let start = out.find(MARK)? + MARK.len();
    let end = out[start..].find(MARK)? + start;
    let path = out[start..end].trim();
    (!path.is_empty()).then(|| path.to_string())
}

/// Where `name` resolves on the current PATH, if anywhere.
pub fn which(name: &str) -> Option<PathBuf> {
    if name.contains('/') {
        return Path::new(name).is_file().then(|| PathBuf::from(name));
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn which_finds_sh_and_not_nonsense() {
        assert!(which("sh").is_some());
        assert!(which("definitely-not-a-command-recalc").is_none());
        assert!(which("/bin/sh").is_some());
        assert!(which("/nonexistent/sh").is_none());
    }

    #[test]
    fn login_shell_path_is_a_path_or_nothing() {
        // Either a colon-separated path or None, never shell banner output.
        if let Some(p) = login_shell_path() {
            assert!(!p.contains(MARK));
            assert!(p.split(':').any(|dir| dir.starts_with('/')), "{p}");
        }
    }

    #[test]
    fn adopting_keeps_every_existing_entry() {
        let before: Vec<PathBuf> =
            std::env::split_paths(&std::env::var_os("PATH").unwrap()).collect();
        adopt_login_shell_path();
        let after: Vec<PathBuf> =
            std::env::split_paths(&std::env::var_os("PATH").unwrap()).collect();
        for p in &before {
            assert!(after.contains(p), "{} was dropped", p.display());
        }
    }
}
