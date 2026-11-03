//! Updates: a check against the release feed once a day (or from Check for
//! Updates…), and Restart to Update. Every update is signed with the key whose
//! public half is in `tauri.conf.json`; one that isn't is refused, whatever
//! feed it came from.
//!
//! The feed is `RECALC_UPDATE_URL`, set when the release workflow builds (the
//! repository's `releases/latest/download/latest.json`) and overridable at
//! run time. A build from source has none and doesn't check.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::board::Shared;

/// The update found by the last check, kept for Restart to Update.
#[derive(Default)]
pub struct Pending(Mutex<Option<Update>>);

/// How long after launch the first automatic check waits, and the gap after.
const FIRST_CHECK: Duration = Duration::from_secs(20);
const EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// The release feed, if this build has one.
pub fn feed() -> Option<String> {
    std::env::var("RECALC_UPDATE_URL")
        .ok()
        .or_else(|| option_env!("RECALC_UPDATE_URL").map(String::from))
        .filter(|u| !u.trim().is_empty())
}

/// The releases page a feed on GitHub belongs to, for "Download" where an
/// update can't install itself.
fn release_page(feed: &str) -> Option<String> {
    let at = feed.find("/releases/")?;
    Some(format!("{}/releases/latest", &feed[..at]))
}

/// Why this copy can't replace itself, if it can't.
fn cannot_install() -> Option<&'static str> {
    let exe = std::env::current_exe().ok()?;
    if crate::board::from_disk_image(&exe) {
        return Some("recalc is running from its disk image. Drag it to Applications, open it from there, and update.");
    }
    if exe.to_string_lossy().contains("/AppTranslocation/") {
        return Some("macOS is running recalc from a temporary copy. Move it to Applications, open it from there, and update.");
    }
    // The updater replaces an AppImage; a .deb belongs to the package manager.
    if cfg!(target_os = "linux") && std::env::var_os("APPIMAGE").is_none() {
        return Some("This copy was installed as a package. Download the new version and install it the same way.");
    }
    None
}

// ---- the person's choice ----

#[derive(Serialize, Deserialize)]
struct Prefs {
    #[serde(default = "yes")]
    check_updates: bool,
}
fn yes() -> bool {
    true
}

fn prefs_path() -> Option<PathBuf> {
    Some(crate::board::config_dir()?.join("settings.json"))
}

fn load_prefs() -> Prefs {
    prefs_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Prefs {
            check_updates: true,
        })
}

fn save_prefs(p: &Prefs) -> Result<(), String> {
    let path = prefs_path().ok_or("no settings folder on this computer")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(p).unwrap()).map_err(|e| e.to_string())
}

// ---- checking ----

/// What the window shows about an update found.
fn describe(update: &Update, feed: &str) -> Value {
    let blocked = cannot_install();
    json!({
        "version": update.version,
        "current": update.current_version,
        "notes": update.body.clone().unwrap_or_default(),
        "date": update.date.map(|d| d.to_string()),
        "can_install": blocked.is_none(),
        "why_not": blocked,
        "page": release_page(feed),
    })
}

async fn check_now(app: &AppHandle) -> Result<Option<Value>, String> {
    let feed = feed()
        .ok_or("This copy of recalc was built from source, so it doesn't check for updates.")?;
    let url = feed
        .parse()
        .map_err(|e| format!("the update address {feed} isn't a URL: {e}"))?;
    let updater = app
        .updater_builder()
        .endpoints(vec![url])
        .map_err(|e| e.to_string())?
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let found = updater
        .check()
        .await
        .map_err(|e| format!("Couldn't check for updates: {e}"))?;
    let shown = found.as_ref().map(|u| describe(u, &feed));
    *app.state::<Pending>().0.lock().unwrap() = found;
    Ok(shown)
}

/// This version, whether checks are on, and whether this build can check.
#[tauri::command]
pub fn update_status(app: AppHandle) -> Value {
    json!({
        "current": app.package_info().version.to_string(),
        "auto": load_prefs().check_updates,
        "feed": feed().is_some(),
    })
}

/// Check now. `null` when this is the newest version.
#[tauri::command]
pub async fn check_update(app: AppHandle) -> Result<Option<Value>, String> {
    check_now(&app).await
}

#[tauri::command]
pub fn set_auto_update(on: bool) -> Result<(), String> {
    save_prefs(&Prefs { check_updates: on })
}

/// Download the update found, install it over this copy, and restart into it.
/// Progress goes to the window as `update-progress` {done, total}.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    if let Some(why) = cannot_install() {
        return Err(why.to_string());
    }
    let update = app
        .state::<Pending>()
        .0
        .lock()
        .unwrap()
        .clone()
        .ok_or("There's no update to install; check again.")?;
    let mut done = 0u64;
    let progress = app.clone();
    update
        .download_and_install(
            move |chunk, total| {
                done += chunk as u64;
                let _ = progress.emit("update-progress", json!({"done": done, "total": total}));
            },
            || {},
        )
        .await
        .map_err(|e| format!("The update didn't install: {e}"))?;
    // A restart skips RunEvent::Exit: stop the runs here, as a quit would.
    app.state::<Shared>().lock().engine.shutdown();
    app.restart();
}

/// Check once a day while the window is open, if the person hasn't turned
/// it off; an update found goes to the window as `update-available`.
pub fn start_checks(app: &AppHandle) {
    if feed().is_none() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        // Tests wait less (RECALC_UPDATE_DELAY_MS).
        let first = std::env::var("RECALC_UPDATE_DELAY_MS")
            .ok()
            .and_then(|ms| ms.parse().ok())
            .map(Duration::from_millis)
            .unwrap_or(FIRST_CHECK);
        tokio::time::sleep(first).await;
        loop {
            if load_prefs().check_updates {
                if let Ok(Some(found)) = check_now(&app).await {
                    let _ = app.emit("update-available", found);
                }
            }
            tokio::time::sleep(EVERY).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_github_feed_names_its_releases_page() {
        assert_eq!(
            release_page("https://github.com/someone/recalc/releases/latest/download/latest.json")
                .as_deref(),
            Some("https://github.com/someone/recalc/releases/latest")
        );
        assert_eq!(release_page("https://example.com/latest.json"), None);
    }

    #[test]
    fn checks_are_on_unless_turned_off() {
        let p: Prefs = serde_json::from_str("{}").unwrap();
        assert!(p.check_updates);
        let p: Prefs = serde_json::from_str(r#"{"check_updates": false}"#).unwrap();
        assert!(!p.check_updates);
    }
}
