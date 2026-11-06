//! MCP server: JSON-RPC 2.0 over newline-delimited JSON on stdio.
//!
//! Blocking tools (await_cell, ask_cell, trigger_cell) are long-polls that
//! answer within timeout_ms; agents re-call until resolved, and ask_cell
//! re-attaches to an identical pending question.
//!
//! Each connection's reads are remembered: when a cell it read changes, the
//! next tool result carries a second content block naming the change.
//!
//! This file dispatches tools; `schema` lists them and checks arguments,
//! `reads` and `writes` implement them, `waits` holds the blocking ones, and
//! `guards` decides what an agent may change.

use crate::engine::{Choice, Engine};
use crate::expr;
use crate::model::{
    a1_name, cell_coord, on_sheet, Actor, AnswerType, Cell, CellKind, Cite, Decision,
    DecisionOption, Mark, MarkKind, Provenance,
};
use crate::store::Watch;
use anyhow::Result;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

mod guards;
mod reads;
mod schema;
mod structure;
#[cfg(test)]
mod tests;
mod waits;
mod writes;

use reads::{get_range, range_names};
use schema::{argument_problem, tool_list};
use writes::{answer_type_targets, cite_arg, Entries};

/// Stays under the ~60s tool-call timeout MCP clients impose.
const DEFAULT_BLOCK_MS: u64 = 45_000;
/// Upper bound on a requested block; must stay under the client's timeout.
const MAX_BLOCK_MS: u64 = 50_000;

/// Board overview sent to every MCP client at initialize.
pub(crate) const INSTRUCTIONS: &str = include_str!("../prompts/instructions.md");

pub struct Server {
    engine: Engine,
    watch: Watch,
    /// Per-connection agent id, from the initialize handshake.
    agents: HashMap<u64, String>,
    /// Config-supplied override (`recalc mcp --id`).
    default_id: Option<String>,
    /// Serving a headless cell agent (`recalc mcp --cell`). It runs without a
    /// shell and reads untrusted documents, so it may not write or run `$`/`!` cells.
    cell_agent: bool,
    waiters: Vec<Waiter>,
    /// Per connection: each cell it read, with the value it last saw.
    reads: HashMap<u64, HashMap<String, String>>,
}

pub struct Outgoing {
    pub conn: u64,
    pub line: String,
}

struct Waiter {
    conn: u64,
    id: Value,
    name: String,
    wait: Wait,
    deadline: Instant,
}

enum Wait {
    Await {
        predicate: Option<String>,
        initial: String,
    },
    Ask,
    Trigger,
}

impl Server {
    pub fn new(dir: &Path, default_id: Option<String>) -> Result<Server> {
        let engine = Engine::open(dir)?;
        let watch = engine.watch()?;
        Ok(Server {
            engine,
            watch,
            agents: HashMap::new(),
            default_id,
            cell_agent: false,
            waiters: Vec::new(),
            reads: HashMap::new(),
        })
    }

    /// This server serves a headless cell agent: no `$`/`!` writes or runs.
    pub fn for_cell_agent(mut self) -> Server {
        self.cell_agent = true;
        self
    }

    /// Absorb board changes and answer any waiters that became ready.
    /// Transports call this on a ~50 ms tick.
    pub fn tick(&mut self) -> Result<Vec<Outgoing>> {
        self.engine.pump(&mut self.watch)?;
        Ok(self.check_waiters())
    }

    pub fn drop_conn(&mut self, conn: u64) {
        self.agents.remove(&conn);
        self.reads.remove(&conn);
        self.waiters.retain(|w| w.conn != conn);
    }

    /// Remember that `conn` saw `name` at its current value.
    fn note_read(&mut self, conn: u64, name: &str) {
        let value = self.value_of(name);
        self.reads
            .entry(conn)
            .or_default()
            .insert(name.to_string(), value);
    }

    /// Cells `conn` read whose value has since changed, each reported once.
    fn changed_inputs(&mut self, conn: u64) -> Vec<Value> {
        let Some(seen) = self.reads.get(&conn) else {
            return Vec::new();
        };
        let mut changed: Vec<(String, String, String)> = seen
            .iter()
            .filter_map(|(name, was)| {
                let now = self.value_of(name);
                (now != *was).then(|| (name.clone(), was.clone(), now))
            })
            .collect();
        changed.sort();
        let seen = self.reads.get_mut(&conn).expect("checked above");
        for (name, _, now) in &changed {
            seen.insert(name.clone(), now.clone());
        }
        changed
            .into_iter()
            .map(|(name, was, now)| json!({"name": name, "was": was, "now": now}))
            .collect()
    }

    pub fn handle_line(&mut self, conn: u64, line: &str) -> Vec<Outgoing> {
        let _ = self.engine.pump(&mut self.watch);
        let Ok(msg) = serde_json::from_str::<Value>(line) else {
            return vec![self.err_out(conn, Value::Null, -32700, "parse error")];
        };
        let method = msg["method"].as_str().unwrap_or_default().to_string();
        let id = msg["id"].clone();
        if id.is_null() {
            return Vec::new(); // notification (e.g. notifications/initialized)
        }
        let params = msg["params"].clone();
        let mut outs = match method.as_str() {
            "initialize" => {
                let client = params["clientInfo"]["name"].as_str().map(str::to_string);
                let agent_id = self
                    .default_id
                    .clone()
                    .or(client)
                    .unwrap_or_else(|| format!("agent-{conn}"));
                self.agents.insert(conn, agent_id);
                vec![self.result_out(conn, id, json!({
                    "protocolVersion": negotiate(params["protocolVersion"].as_str()),
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "recalc", "version": env!("CARGO_PKG_VERSION") },
                    // Sent whether or not the repo has AGENTS.md.
                    "instructions": self.engine.prompt_override("instructions").unwrap_or_else(|| INSTRUCTIONS.to_string()).trim(),
                }))]
            }
            "ping" => vec![self.result_out(conn, id, json!({}))],
            "tools/list" => vec![self.result_out(conn, id, json!({ "tools": tool_list() }))],
            "tools/call" => self.call_tool(conn, id, &params),
            _ => vec![self.err_out(conn, id, -32601, &format!("unknown method {method}"))],
        };
        outs.extend(self.check_waiters());
        outs
    }

    /// Cell names in a call however the agent wrote them (`'My Notes'!A2`,
    /// `notes!A2`, `Sheet1!A2`), as the board names them, so every tool
    /// below sees one spelling.
    fn canonical_args(&self, mut args: Value) -> Value {
        for key in ["name", "from", "to", "at"] {
            if let Some(v) = args[key].as_str() {
                args[key] = json!(self.engine.canonical(v));
            }
        }
        if let Some(v) = args["range"].as_str() {
            args["range"] = json!(self.engine.canonical_range(v));
        }
        if let Some(names) = args["names"].as_array() {
            let names: Vec<Value> = names
                .iter()
                .map(|n| {
                    n.as_str()
                        .map_or(n.clone(), |s| json!(self.engine.canonical(s)))
                })
                .collect();
            args["names"] = json!(names);
        }
        for key in ["cells", "cites"] {
            if let Some(map) = args[key].as_object() {
                let map: serde_json::Map<String, Value> = map
                    .iter()
                    .map(|(k, v)| (self.engine.canonical(k), v.clone()))
                    .collect();
                args[key] = Value::Object(map);
            }
        }
        args
    }

    fn agent(&self, conn: u64) -> Actor {
        match self.agents.get(&conn) {
            Some(id) => Actor::Agent(id.clone()),
            None => Actor::Agent(format!("agent-{conn}")),
        }
    }

    fn call_tool(&mut self, conn: u64, id: Value, params: &Value) -> Vec<Outgoing> {
        let tool = params["name"].as_str().unwrap_or_default().to_string();
        let args = self.canonical_args(params["arguments"].clone());
        let name = args["name"].as_str().unwrap_or_default().to_string();
        let by = self.agent(conn);
        if let Some(e) = argument_problem(&tool, &args) {
            return vec![self.tool_out(conn, id, Err(e))];
        }
        if let Some(e) = self.shell_refusal(&tool, &args) {
            return vec![self.tool_out(conn, id, Err(e))];
        }

        let result: std::result::Result<Value, String> = match tool.as_str() {
            "list_cells" => Ok(self.list_cells()),
            "get_cell" => {
                let r = self.get_cell(&name);
                if r.is_ok() {
                    self.note_read(conn, &name);
                }
                r
            }
            "set_cell" => {
                let answered = self.answered();
                let content = args["content"].as_str().unwrap_or_default();
                let note = args["note"].as_str().map(str::to_string);
                if let Err(e) = self.guard_unchanged(conn, &args["if_unchanged"]) {
                    return vec![self.tool_out(conn, id, Err(e))];
                }
                if let Some(why) = self.outside_its_cell(&name) {
                    let r = self
                        .engine
                        .suggest(&name, content, by, note, &why)
                        .map(|()| json!({"ok": true, "held_as_suggestion": format!("{why}; a person decides whether to adopt it")}))
                        .map_err(|e| e.to_string());
                    return vec![self.tool_out(conn, id, r)];
                }
                let before = self.value_of(&name);
                let r = self
                    .engine
                    .set_noted(&name, content, by.clone(), note)
                    .map_err(|e| e.to_string())
                    .and_then(|()| match cite_arg(&args["cite"], by) {
                        // The write became a suggestion; the cite describes
                        // it, not the verified value.
                        Some(_) if self.verified(&name) => Ok(()),
                        Some(cite) => self
                            .engine
                            .set_cite(&name, Some(cite))
                            .map_err(|e| e.to_string()),
                        None => Ok(()),
                    })
                    .map(|()| {
                        let mut out = self.write_receipt(&name, content, &before);
                        self.note_missing_quote(&name, &mut out);
                        self.note_made_stale(&answered, &mut out);
                        out
                    });
                // Its own write is not news to it.
                self.forget_if_read(conn, &name);
                r
            }
            "cite_cell" => self
                .refuse_if_verified(&name, "citation")
                .and_then(|()| self.refuse_outside(std::slice::from_ref(&name), "citation"))
                .and_then(|()| self.cite_cell(&args, by)),
            "get_range" => {
                let r = get_range(&self.engine, &args);
                if r.is_ok() {
                    for name in range_names(&args) {
                        self.note_read(conn, &name);
                    }
                }
                r
            }
            "set_range" => {
                if let Err(e) = self.guard_unchanged(conn, &args["if_unchanged"]) {
                    return vec![self.tool_out(conn, id, Err(e))];
                }
                let answered = self.answered();
                let r = self.set_range(&args, by).map(|mut out| {
                    self.note_made_stale(&answered, &mut out);
                    out
                });
                for name in range_names(&args) {
                    self.forget_if_read(conn, &name);
                }
                r
            }
            "mark_cell" => self
                .refuse_if_verified(&name, "mark")
                .and_then(|()| self.refuse_outside(std::slice::from_ref(&name), "mark"))
                .and_then(|()| self.mark_cell(&args, by)),
            "set_answer_type" => self
                .refuse_outside(&answer_type_targets(&args), "answer type")
                .and_then(|()| self.set_answer_type(&args)),
            "open_session" => self.request("open_session", &name, true),
            "show_cell" => self.request("show_cell", &name, false),
            "append_cell" => {
                let value = args["value"].as_str().unwrap_or_default();
                if let Err(e) = self.refuse_outside(std::slice::from_ref(&name), "value") {
                    return vec![self.tool_out(conn, id, Err(e))];
                }
                self.engine
                    .set(&name, value, by)
                    .map(|()| json!({"ok": true}))
                    .map_err(|e| e.to_string())
            }
            "history" => {
                let limit = args["limit"].as_u64().unwrap_or(50) as usize;
                self.engine
                    .history(&name, limit)
                    .map(|entries| {
                        json!(entries
                            .into_iter()
                            .map(|(at, value)| json!({"at": at, "value": value}))
                            .collect::<Vec<_>>())
                    })
                    .map_err(|e| e.to_string())
            }
            "trigger_cell" => return self.trigger_and_wait(conn, id, name, by),
            "fill_range" => self.fill_range(&args, by),
            "edit_structure" => self.edit_structure(&args, by),
            "sort_rows" => self.sort_rows(&args, by),
            "rename_sheet" => self.rename_sheet(&args),
            "duplicate_sheet" => self.duplicate_sheet(&args, by),
            "stop_cell" => self.stop_cell(&name, by),
            "set_format" => self.set_format(&args),
            "answer_cell" => {
                // A cell agent answers its own cell with set_cell; others' are theirs.
                if let Some(why) = self.outside_its_cell(&name) {
                    Err(format!(
                        "{why}; an agent computing one cell can't answer another"
                    ))
                } else {
                    let value = args["value"].as_str().unwrap_or_default();
                    let note = args["note"].as_str().map(str::to_string);
                    let before = self.value_of(&name);
                    self.engine
                        .answer_cell(&name, value, by.clone(), note)
                        .and_then(|()| match cite_arg(&args["cite"], by.clone()) {
                            Some(cite) if !self.verified(&name) => {
                                self.engine.set_cite(&name, Some(cite))
                            }
                            _ => Ok(()),
                        })
                        .map_err(|e| e.to_string())
                        .map(|()| {
                            let mut out = self.write_receipt(&name, value, &before);
                            self.note_missing_quote(&name, &mut out);
                            out
                        })
                }
            }
            "run_range" => {
                // Queue and return, so a table's cells run max_agents at a time.
                let names = range_names(&args);
                if names.is_empty() {
                    Err("run_range needs from and to, e.g. B2 and L6".to_string())
                } else {
                    let all = args["all"].as_bool().unwrap_or(false);
                    let out = self.engine.trigger_many(&names, by, all);
                    Ok(json!({
                        "queued": out.ran,
                        "up_to_date": out.up_to_date,
                        "need_a_person": out.need_confirm,
                        "hint": "they run max_agents at a time, each after the cells it reads; get_range to see them land, or await_cell on the last",
                    }))
                }
            }
            "ask_agent" => return self.ask_agent(conn, id, &args, by),
            "await_cell" => return self.await_cell(conn, id, &args),
            "ask_cell" => return self.ask_cell(conn, id, &args),
            "resolve_decision" => self.resolve(&args, by),
            _ => Err(format!("unknown tool {tool}")),
        };
        vec![self.tool_out(conn, id, result)]
    }

    /// Re-baseline cells `conn` just wrote so its own write isn't reported back.
    fn forget_if_read(&mut self, conn: u64, name: &str) {
        if self.reads.get(&conn).is_some_and(|r| r.contains_key(name)) {
            self.note_read(conn, name);
        }
    }

    /// The `if_unchanged` write guard: `true` refuses if any cell this
    /// connection read has moved; a list of names refuses if one of those has.
    /// The refusal carries the changes, which are then re-baselined.
    fn guard_unchanged(&mut self, conn: u64, arg: &Value) -> std::result::Result<(), String> {
        let only: Option<Vec<String>> = match arg {
            Value::Null | Value::Bool(false) => return Ok(()),
            Value::Bool(true) => None,
            Value::Array(names) => Some(
                names
                    .iter()
                    .filter_map(|n| n.as_str().map(str::to_string))
                    .collect(),
            ),
            Value::String(name) => Some(vec![name.clone()]),
            _ => return Err("if_unchanged must be true or a list of cell names".into()),
        };
        // A guard over an unread cell guards nothing; refuse it.
        let seen = self.reads.get(&conn);
        let unread: Vec<&String> = match &only {
            Some(names) => names
                .iter()
                .filter(|n| !seen.is_some_and(|r| r.contains_key(*n)))
                .collect(),
            None => Vec::new(),
        };
        if !unread.is_empty() {
            let list: Vec<&str> = unread.iter().map(|n| n.as_str()).collect();
            return Err(format!(
                "if_unchanged names {} you haven't read on this connection; get_cell it first, so there is a value to compare",
                list.join(", ")
            ));
        }
        let Some(seen) = seen.filter(|r| !r.is_empty()) else {
            return Err("if_unchanged: true, but you haven't read any cells yet; read the cells the write rests on first".into());
        };
        let mut moved: Vec<(String, String, String)> = seen
            .iter()
            .filter(|(name, _)| only.as_ref().is_none_or(|o| o.contains(name)))
            .filter_map(|(name, was)| {
                let now = self.value_of(name);
                (now != *was).then(|| (name.clone(), was.clone(), now))
            })
            .collect();
        if moved.is_empty() {
            return Ok(());
        }
        moved.sort();
        let seen = self.reads.get_mut(&conn).expect("checked above");
        for (name, _, now) in &moved {
            seen.insert(name.clone(), now.clone());
        }
        let changed: Vec<Value> = moved
            .into_iter()
            .map(|(name, was, now)| json!({"name": name, "was": was, "now": now}))
            .collect();
        Err(json!({
            "refused": "inputs changed since you read them; re-read and answer again",
            "changed_inputs": changed,
        })
        .to_string())
    }

    fn value_of(&self, name: &str) -> String {
        self.engine
            .cell(name)
            .map(|c| c.value.clone())
            .unwrap_or_default()
    }

    // ---- JSON-RPC plumbing ----

    fn result_out(&self, conn: u64, id: Value, result: Value) -> Outgoing {
        Outgoing {
            conn,
            line: json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string(),
        }
    }

    fn err_out(&self, conn: u64, id: Value, code: i64, message: &str) -> Outgoing {
        Outgoing {
            conn,
            line: json!({
                "jsonrpc": "2.0", "id": id,
                "error": {"code": code, "message": message},
            })
            .to_string(),
        }
    }

    /// Tool results are text content; tool-level failures use isError rather
    /// than a protocol error. A successful result gains a second text block when
    /// cells this connection read have changed since.
    fn tool_out(
        &mut self,
        conn: u64,
        id: Value,
        result: std::result::Result<Value, String>,
    ) -> Outgoing {
        let body = match result {
            Ok(v) => {
                let mut content = vec![json!({"type": "text", "text": v.to_string()})];
                let changed = self.changed_inputs(conn);
                if !changed.is_empty() {
                    content.push(json!({
                        "type": "text",
                        "text": json!({"changed_inputs": changed}).to_string(),
                    }));
                }
                json!({"content": content})
            }
            Err(msg) => json!({
                "content": [{"type": "text", "text": msg}],
                "isError": true,
            }),
        };
        self.result_out(conn, id, body)
    }
}

/// Protocol versions this server speaks. A client asking for another gets
/// the newest and decides whether to continue.
const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

fn negotiate(requested: Option<&str>) -> &'static str {
    PROTOCOL_VERSIONS
        .iter()
        .find(|v| Some(**v) == requested)
        .unwrap_or(&PROTOCOL_VERSIONS[0])
}

/// `recalc mcp`: serve one client over stdio.
pub async fn serve_stdio(
    dir: &Path,
    default_id: Option<String>,
    cell_agent: bool,
    depth: u32,
) -> Result<()> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let mut server = Server::new(dir, default_id)?;
    server.engine.set_depth(depth);
    server.engine.set_hand_off_runs(true);
    if cell_agent {
        server = server.for_cell_agent();
    }
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();
    let mut tick = tokio::time::interval(Duration::from_millis(50));
    // Claude Code sends SIGINT, SIGTERM, then SIGKILL before closing stdin;
    // return on the first so the engine's shutdown runs before the kill.
    let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    loop {
        let outs = tokio::select! {
            _ = sigint.recv() => return Ok(()),
            _ = sigterm.recv() => return Ok(()),
            line = lines.next_line() => match line? {
                Some(l) => server.handle_line(0, &l),
                None => {
                    // Client gone: finish the runs this process started, or they die with it.
                    let deadline = std::time::Instant::now()
                        + Duration::from_millis(server.engine.agent_timeout_ms().max(60_000));
                    while server.engine.runs_in_progress() && std::time::Instant::now() < deadline {
                        server.tick()?;
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                    return Ok(());
                }
            },
            _ = tick.tick() => server.tick()?,
        };
        for out in outs {
            stdout.write_all(out.line.as_bytes()).await?;
            stdout.write_all(b"\n").await?;
        }
        stdout.flush().await?;
    }
}
