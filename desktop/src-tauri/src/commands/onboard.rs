//! A first run that says what's missing: whether Claude Code, Codex and pi are
//! installed and signed in, and a `recalc` command on the PATH a terminal
//! sees (Install Command Line Tool…), as VS Code does for `code`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};
use tauri::State;

use crate::board::Shared;

// ---- agents ----

/// Each built-in agent: where it resolves, and whether it's signed in
/// (`null` when its CLI couldn't say).
#[tauri::command]
pub(crate) async fn agent_status(state: State<'_, Shared>) -> Result<Vec<Value>, String> {
    let paths = state.lock().engine.config().paths.clone();
    let mut out = Vec::new();
    for (name, product, install, status) in [
        (
            "claude",
            "Claude Code",
            "https://claude.com/claude-code",
            &["auth", "status"] as &[&str],
        ),
        (
            "codex",
            "Codex",
            "https://github.com/openai/codex",
            &["login", "status"],
        ),
        // pi signs in per provider and has no one command that says so.
        ("pi", "pi", "https://pi.dev", &[]),
    ] {
        let wanted = paths.get(name).cloned().unwrap_or_else(|| name.to_string());
        let found = recalc::shell_env::which(&wanted);
        let signed_in = match &found {
            Some(exe) if !status.is_empty() => signed_in(exe, status).await,
            _ => None,
        };
        out.push(json!({
            "name": name,
            "product": product,
            "found": found.map(|p| p.display().to_string()),
            "signed_in": signed_in,
            "install": install,
        }));
    }
    Ok(out)
}

/// Ask the CLI. Both exit 0 when signed in and 1 when not; Claude Code also
/// prints `"loggedIn"`. Anything else (an old CLI without the subcommand, a
/// hang) is "can't tell", which the window doesn't warn about.
async fn signed_in(exe: &Path, args: &[&str]) -> Option<bool> {
    // Not in the board's folder: a CLI that writes where it started could
    // recreate a folder the board has just moved out of.
    let run = tokio::process::Command::new(exe)
        .args(args)
        .current_dir(dirs::home_dir().unwrap_or_else(std::env::temp_dir))
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(Duration::from_secs(8), run)
        .await
        .ok()?
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).to_lowercase()
        + &String::from_utf8_lossy(&out.stderr).to_lowercase();
    if text.contains("\"loggedin\": true")
        || (out.status.success() && !text.contains("not logged in"))
    {
        Some(true)
    } else if text.contains("\"loggedin\": false") || text.contains("not logged in") {
        Some(false)
    } else {
        None
    }
}

// ---- the command line tool ----

/// Where the `recalc` command goes: a folder a terminal's PATH has, that the
/// person can write without a password if there is one.
#[derive(Debug, PartialEq)]
struct Target {
    dir: PathBuf,
    on_path: bool,
    needs_admin: bool,
}

fn choose_target(login_path: &[PathBuf], home: &Path, writable: impl Fn(&Path) -> bool) -> Target {
    let local = home.join(".local/bin");
    let usr = PathBuf::from("/usr/local/bin");
    if login_path.contains(&local) {
        return Target {
            dir: local,
            on_path: true,
            needs_admin: false,
        };
    }
    if login_path.contains(&usr) && writable(&usr) {
        return Target {
            dir: usr,
            on_path: true,
            needs_admin: false,
        };
    }
    // macOS puts /usr/local/bin on every PATH (/etc/paths), even before it exists.
    if cfg!(target_os = "macos") {
        let needs_admin = !writable(&usr);
        return Target {
            dir: usr,
            on_path: true,
            needs_admin,
        };
    }
    Target {
        dir: local,
        on_path: false,
        needs_admin: false,
    }
}

fn writable(dir: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else {
        return false;
    };
    unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
}

fn login_path() -> Vec<PathBuf> {
    recalc::shell_env::login_shell_path()
        .or_else(|| std::env::var("PATH").ok())
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default()
}

fn same_file(a: &Path, b: &Path) -> bool {
    matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
}

/// What Install Command Line Tool would do: where it puts `recalc`, and any
/// `recalc` a terminal already finds (installed when that one is this app).
#[tauri::command]
pub(crate) async fn cli_status() -> Result<Value, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let home = dirs::home_dir().ok_or("no home folder")?;
    let path = tauri::async_runtime::spawn_blocking(login_path)
        .await
        .map_err(|e| e.to_string())?;
    let target = choose_target(&path, &home, writable);
    let existing = path
        .iter()
        .map(|d| d.join("recalc"))
        .find(|p| p.symlink_metadata().is_ok());
    let installed = existing.as_deref().is_some_and(|p| same_file(p, &exe));
    Ok(json!({
        "exe": exe.display().to_string(),
        "target": target.dir.join("recalc").display().to_string(),
        "on_path": target.on_path,
        "needs_admin": target.needs_admin,
        "installed": installed,
        "existing": existing.as_ref().map(|p| json!({
            "path": p.display().to_string(),
            "link_to": std::fs::read_link(p).ok().map(|l| l.display().to_string()),
        })),
        "blocked": blocked(&exe),
    }))
}

/// A copy that will move or vanish can't be linked to.
fn blocked(exe: &Path) -> Option<&'static str> {
    if crate::board::from_disk_image(exe) {
        return Some("recalc is running from its disk image. Drag it to Applications, open it from there, and install the command then.");
    }
    if exe.to_string_lossy().contains("/AppTranslocation/") {
        return Some("macOS is running recalc from a temporary copy. Move it to Applications, open it from there, and install the command then.");
    }
    None
}

/// Link `recalc` to this app. An existing `recalc` a terminal would find
/// first is replaced only when it's a link and `replace` is set; a real file
/// is never removed.
#[tauri::command]
pub(crate) async fn install_cli(replace: bool) -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    if let Some(why) = blocked(&exe) {
        return Err(why.to_string());
    }
    let home = dirs::home_dir().ok_or("no home folder")?;
    let path = tauri::async_runtime::spawn_blocking(login_path)
        .await
        .map_err(|e| e.to_string())?;
    let mut target = choose_target(&path, &home, writable);
    let mut link = target.dir.join("recalc");
    // A `recalc` earlier on the PATH would win over a new one: replace that one.
    if let Some(found) = path
        .iter()
        .map(|d| d.join("recalc"))
        .find(|p| p.symlink_metadata().is_ok())
    {
        if same_file(&found, &exe) {
            return Ok(found.display().to_string());
        }
        let is_link = found
            .symlink_metadata()
            .is_ok_and(|m| m.file_type().is_symlink());
        if !is_link {
            return Err(format!(
                "There's already a recalc program at {}. Remove or rename it, then try again.",
                found.display()
            ));
        }
        if !replace {
            return Err(format!("There's already a recalc at {}.", found.display()));
        }
        let dir = found.parent().map(Path::to_path_buf).unwrap_or_default();
        target = Target {
            needs_admin: !writable(&dir),
            dir,
            on_path: true,
        };
        link = found;
    }
    if target.needs_admin {
        link_as_admin(&exe, &link)?;
    } else {
        std::fs::create_dir_all(&target.dir)
            .map_err(|e| format!("couldn't make {}: {e}", target.dir.display()))?;
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&exe, &link)
            .map_err(|e| format!("couldn't link {}: {e}", link.display()))?;
    }
    Ok(link.display().to_string())
}

/// macOS's own password prompt, as installers use, for a folder only an
/// administrator can write.
#[cfg(target_os = "macos")]
fn link_as_admin(exe: &Path, link: &Path) -> Result<(), String> {
    let sh = |p: &Path| recalc::interp::shell_quote(&p.display().to_string());
    let dir = link.parent().unwrap_or(Path::new("/usr/local/bin"));
    let script = format!("mkdir -p {} && ln -sfn {} {}", sh(dir), sh(exe), sh(link));
    let apple = script.replace('\\', "\\\\").replace('"', "\\\"");
    let out = std::process::Command::new("osascript")
        .args([
            "-e",
            &format!("do shell script \"{apple}\" with administrator privileges"),
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else if String::from_utf8_lossy(&out.stderr).contains("-128") {
        Err("Cancelled.".into())
    } else {
        Err(format!(
            "couldn't link {}: {}",
            link.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

#[cfg(not(target_os = "macos"))]
fn link_as_admin(_exe: &Path, link: &Path) -> Result<(), String> {
    Err(format!(
        "{} needs an administrator; link it with sudo ln -s",
        link.display()
    ))
}

/// Open a web page in the person's browser: only https, since the address
/// can come from a release feed.
#[tauri::command]
pub(crate) fn open_url(url: String) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err(format!("not opening {url}: only https addresses"));
    }
    #[cfg(target_os = "macos")]
    let mut cmd = std::process::Command::new("open");
    #[cfg(not(target_os = "macos"))]
    let mut cmd = std::process::Command::new("xdg-open");
    cmd.arg(&url)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("couldn't open {url}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_goes_where_a_terminal_looks_without_a_password_when_it_can() {
        let home = Path::new("/Users/me");
        let local = home.join(".local/bin");
        let usr = PathBuf::from("/usr/local/bin");
        // ~/.local/bin on the PATH (Claude Code's installer puts it there): no password.
        let t = choose_target(&[usr.clone(), local.clone()], home, |_| false);
        assert_eq!(
            t,
            Target {
                dir: local.clone(),
                on_path: true,
                needs_admin: false
            }
        );
        // A writable /usr/local/bin (Intel Homebrew owns it).
        let t = choose_target(std::slice::from_ref(&usr), home, |_| true);
        assert_eq!(
            t,
            Target {
                dir: usr.clone(),
                on_path: true,
                needs_admin: false
            }
        );
        // Neither: macOS asks for the password; elsewhere ~/.local/bin, and the window says to add it.
        let t = choose_target(&[PathBuf::from("/usr/bin")], home, |_| false);
        if cfg!(target_os = "macos") {
            assert_eq!(
                t,
                Target {
                    dir: usr,
                    on_path: true,
                    needs_admin: true
                }
            );
        } else {
            assert_eq!(
                t,
                Target {
                    dir: local,
                    on_path: false,
                    needs_admin: false
                }
            );
        }
    }

    #[test]
    fn a_link_to_another_build_isnt_this_app() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("app-recalc");
        std::fs::write(&exe, "").unwrap();
        let other = tmp.path().join("other-recalc");
        std::fs::write(&other, "").unwrap();
        let link = tmp.path().join("recalc");
        std::os::unix::fs::symlink(&other, &link).unwrap();
        assert!(
            !same_file(&link, &exe),
            "a link to another build isn't this app"
        );
        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(&exe, &link).unwrap();
        assert!(same_file(&link, &exe));
    }
}
