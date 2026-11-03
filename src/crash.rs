//! Panic logging. Launched from the Dock or Finder, stderr goes nowhere, so
//! each panic and its backtrace is also appended to a log file.

use std::io::Write;
use std::path::PathBuf;

/// `~/Library/Logs/recalc/crash.log` on macOS, `$XDG_STATE_HOME/recalc/`
/// (or `~/.local/state/recalc/`) elsewhere.
pub fn log_path() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let dir = if cfg!(target_os = "macos") {
        home.join("Library/Logs/recalc")
    } else {
        std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/state"))
            .join("recalc")
    };
    Some(dir.join("crash.log"))
}

/// Install the panic hook. The default hook still runs afterwards.
pub fn install() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(path) = log_path() {
            let _ = write_entry(&path, &info.to_string());
        }
        default(info);
    }));
}

fn write_entry(path: &std::path::Path, what: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let thread = std::thread::current();
    writeln!(
        f,
        "--- recalc {} panicked at unix time {secs} on thread {}\n{what}\n{}",
        env!("CARGO_PKG_VERSION"),
        thread.name().unwrap_or("unnamed"),
        std::backtrace::Backtrace::force_capture()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entry_carries_the_message_and_a_backtrace() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/crash.log");
        write_entry(&path, "boom at src/x.rs:1").unwrap();
        write_entry(&path, "again").unwrap();
        let log = std::fs::read_to_string(&path).unwrap();
        assert_eq!(log.matches("--- recalc ").count(), 2);
        assert!(log.contains("boom at src/x.rs:1"));
        assert!(log.contains("an_entry_carries_the_message_and_a_backtrace"));
    }
}
