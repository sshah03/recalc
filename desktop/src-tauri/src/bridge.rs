//! Bridge mode: drive the real app from a browser (debug builds only).
//!
//! macOS's webview has no automation driver. With `RECALC_BRIDGE=<port>` the
//! window is hidden and the frontend is served on `http://127.0.0.1:<port>`
//! with a stand-in `window.__TAURI__`: each `invoke` is posted here and run in
//! the hidden webview through the real IPC, and app events return over SSE.
//!
//! Endpoints: `GET /…` the frontend (with `/__bridge.js`), `POST /invoke`
//! `{cmd, args}`, `GET /events` (SSE), `POST /pick` `{path}` to answer the
//! next file dialog, `GET /dialogs` how many file dialogs the app has asked
//! for, `POST /menu` `{id}`, `POST /quit`, `GET /ready`.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Listener, Manager};

/// The events the frontend listens for; each is forwarded to the browser.
const EVENTS: &[&str] = &[
    "board",
    "menu",
    "pty-data",
    "pty-exit",
    "show-cell",
    "show-session",
    "update-available",
    "update-progress",
];

/// The port, when bridge mode is on.
pub fn port() -> Option<u16> {
    std::env::var("RECALC_BRIDGE").ok()?.parse().ok()
}

struct State {
    replies: Mutex<HashMap<u64, mpsc::Sender<Result<Value, String>>>>,
    clients: Mutex<Vec<TcpStream>>,
    picks: Mutex<Vec<PathBuf>>,
    /// File dialogs asked for, answered by a pick or not.
    dialogs: AtomicU64,
    next: AtomicU64,
    /// The hidden page has loaded and can run calls.
    ready: std::sync::atomic::AtomicBool,
}

fn state() -> &'static State {
    static S: OnceLock<State> = OnceLock::new();
    S.get_or_init(|| State {
        replies: Mutex::new(HashMap::new()),
        clients: Mutex::new(Vec::new()),
        picks: Mutex::new(Vec::new()),
        dialogs: AtomicU64::new(0),
        next: AtomicU64::new(1),
        ready: std::sync::atomic::AtomicBool::new(false),
    })
}

/// The path a test queued for the next file dialog, if any.
pub fn take_pick() -> Option<PathBuf> {
    port()?;
    state().dialogs.fetch_add(1, Ordering::SeqCst);
    let mut picks = state().picks.lock().ok()?;
    (!picks.is_empty()).then(|| picks.remove(0))
}

/// The hidden webview's answer to a forwarded `invoke`; id 0 means the page is ready.
#[tauri::command]
pub fn bridge_reply(id: u64, ok: Option<Value>, err: Option<String>) {
    if id == 0 {
        state().ready.store(true, Ordering::SeqCst);
        return;
    }
    let tx = state().replies.lock().unwrap().remove(&id);
    if let Some(tx) = tx {
        let _ = tx.send(match err {
            Some(e) => Err(e),
            None => Ok(ok.unwrap_or(Value::Null)),
        });
    }
}

/// Hide the window, forward events, and serve. Called from setup.
pub fn start(app: &AppHandle, port: u16) {
    // The window's own page would run the app a second time; it hosts only the IPC.
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
        if let Ok(url) = w.url().and_then(|u| {
            u.join("bridge-host.html")
                .map_err(|e| tauri::Error::Anyhow(e.into()))
        }) {
            let _ = w.navigate(url);
        }
    }
    for &name in EVENTS {
        app.listen_any(name, move |ev| {
            let line = format!(
                "data: {}\n\n",
                json!({"event": name, "payload": serde_json::from_str::<Value>(ev.payload()).unwrap_or(Value::Null)})
            );
            state()
                .clients
                .lock()
                .unwrap()
                .retain_mut(|c| c.write_all(line.as_bytes()).and_then(|()| c.flush()).is_ok());
        });
    }
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("recalc: bridge can't listen on {port}: {e}");
            std::process::exit(2);
        }
    };
    eprintln!("recalc: bridge on http://127.0.0.1:{port}");
    let app = app.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let app = app.clone();
            std::thread::spawn(move || {
                let _ = serve(&app, stream);
            });
        }
    });
}

fn serve(app: &AppHandle, mut stream: TcpStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut first = String::new();
    reader.read_line(&mut first)?;
    let mut parts = first.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or("/"));
    let mut length = 0usize;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let l = line.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some(v) = l.to_ascii_lowercase().strip_prefix("content-length:") {
            length = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    let path = path.split('?').next().unwrap_or("/");
    match (method, path) {
        ("GET", "/events") => {
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\n\r\n")?;
            stream.flush()?;
            state().clients.lock().unwrap().push(stream);
            Ok(())
        }
        // Quit the way the app does, so the engine takes its runs down.
        ("POST", "/quit") => {
            reply(&mut stream, "200 OK", "application/json", b"{\"ok\":true}")?;
            app.exit(0);
            Ok(())
        }
        ("GET", "/ready") if state().ready.load(Ordering::SeqCst) => {
            reply(&mut stream, "200 OK", "application/json", b"{\"ok\":true}")
        }
        ("GET", "/ready") => reply(
            &mut stream,
            "503 Service Unavailable",
            "application/json",
            b"{\"ok\":false}",
        ),
        // Emit the id as the native menu would.
        ("POST", "/menu") => {
            let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            if let Some(id) = v["id"].as_str() {
                let _ = tauri::Emitter::emit(app, "menu", id.to_string());
            }
            reply(&mut stream, "200 OK", "application/json", b"{\"ok\":true}")
        }
        ("GET", "/dialogs") => {
            let n = state().dialogs.load(Ordering::SeqCst);
            reply(
                &mut stream,
                "200 OK",
                "application/json",
                format!("{{\"count\":{n}}}").as_bytes(),
            )
        }
        ("POST", "/pick") => {
            let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            if let Some(p) = v["path"].as_str() {
                state().picks.lock().unwrap().push(PathBuf::from(p));
            }
            reply(&mut stream, "200 OK", "application/json", b"{\"ok\":true}")
        }
        ("POST", "/invoke") => {
            let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            let out = invoke(app, v["cmd"].as_str().unwrap_or_default(), &v["args"]);
            let body = match out {
                Ok(v) => json!({"ok": v}),
                Err(e) => json!({"err": e}),
            };
            reply(
                &mut stream,
                "200 OK",
                "application/json",
                body.to_string().as_bytes(),
            )
        }
        ("GET", _) => {
            let rel = if path == "/" {
                "index.html"
            } else {
                path.trim_start_matches('/')
            };
            if rel == "__bridge.js" {
                return reply(&mut stream, "200 OK", "text/javascript", SHIM.as_bytes());
            }
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../frontend");
            if rel.split('/').any(|c| c == "..") {
                return reply(&mut stream, "403 Forbidden", "text/plain", b"no");
            }
            match std::fs::read(root.join(rel)) {
                Ok(mut bytes) => {
                    if rel == "index.html" {
                        let html = String::from_utf8_lossy(&bytes).replace(
                            "<script src=\"vendor/xterm/xterm.js\">",
                            "<script src=\"/__bridge.js\"></script>\n    <script src=\"vendor/xterm/xterm.js\">",
                        );
                        bytes = html.into_bytes();
                    }
                    reply(&mut stream, "200 OK", content_type(rel), &bytes)
                }
                Err(_) => reply(&mut stream, "404 Not Found", "text/plain", b"not found"),
            }
        }
        _ => reply(&mut stream, "405 Method Not Allowed", "text/plain", b"no"),
    }
}

/// Run `cmd` in the hidden webview through the real IPC and wait for it.
fn invoke(app: &AppHandle, cmd: &str, args: &Value) -> Result<Value, String> {
    let webview = app.get_webview_window("main").ok_or("no webview")?;
    let id = state().next.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = mpsc::channel();
    state().replies.lock().unwrap().insert(id, tx);
    let args = if args.is_null() {
        json!({})
    } else {
        args.clone()
    };
    let js = format!(
        "window.__TAURI__.core.invoke({cmd}, {args}).then(r => window.__TAURI__.core.invoke('bridge_reply', {{id: {id}, ok: r === undefined ? null : r}}), e => window.__TAURI__.core.invoke('bridge_reply', {{id: {id}, err: String(e)}}))",
        cmd = json!(cmd),
    );
    webview.eval(&js).map_err(|e| e.to_string())?;
    // Covers the slowest human-started command; agent runs return once queued.
    rx.recv_timeout(Duration::from_secs(120))
        .map_err(|_| format!("{cmd}: no reply from the webview"))?
}

fn reply(stream: &mut TcpStream, status: &str, kind: &str, body: &[u8]) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript",
        Some("css") => "text/css",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        _ => "application/octet-stream",
    }
}

/// `window.__TAURI__` for the browser: `invoke` posts, `listen` reads SSE.
const SHIM: &str = r#"
(() => {
  const handlers = {};
  const source = new EventSource("/events");
  source.onmessage = (m) => {
    const { event, payload } = JSON.parse(m.data);
    for (const h of handlers[event] || []) h({ event, payload });
  };
  window.__TAURI__ = {
    core: {
      invoke: async (cmd, args) => {
        const r = await fetch("/invoke", { method: "POST", body: JSON.stringify({ cmd, args: args ?? {} }) });
        const out = await r.json();
        if ("err" in out) throw out.err;
        return out.ok;
      },
    },
    event: {
      listen: async (event, cb) => {
        (handlers[event] ||= []).push(cb);
        return () => { handlers[event] = handlers[event].filter((h) => h !== cb); };
      },
    },
  };
  window.__recalcBridge = true;
})();
"#;
