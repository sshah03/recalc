//! macOS window chrome. The toolbar is the page's own, so on its own the
//! window gets a plain title bar's chrome: traffic lights pinned near the top,
//! unlike document apps with a toolbar (Numbers, Terminal). An empty native
//! toolbar asks for that chrome; the system then places the lights and rounds
//! the corners as it does for those apps, on whichever macOS this is.

#[cfg(target_os = "macos")]
pub fn native_toolbar(window: &tauri::WebviewWindow) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSToolbar, NSWindow, NSWindowToolbarStyle};
    // AppKit wants the main thread; setup runs there.
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Ok(ptr) = window.ns_window() else {
        return;
    };
    // SAFETY: Tauri hands back this window's live NSWindow.
    let ns_window: &NSWindow = unsafe { &*ptr.cast::<NSWindow>() };
    let toolbar = NSToolbar::new(mtm);
    ns_window.setToolbar(Some(&toolbar));
    ns_window.setToolbarStyle(NSWindowToolbarStyle::UnifiedCompact);
}

#[cfg(not(target_os = "macos"))]
pub fn native_toolbar(_window: &tauri::WebviewWindow) {}
