//! The `.recalc/` directory: the only module that knows the on-disk layout.
//!
//! ```text
//! .recalc/
//!   cells/<name>       # current VALUE (bare file → `cat cells/x` gives the value)
//!   defs/<name>        # SOURCE, present only for non-literal cells
//!   meta/<name>.toml   # kind, state, provenance
//! ```
//!
//! Values and defs are written with one trailing newline and read with exactly
//! one stripped, so any value round-trips.

use crate::model::{
    on_sheet, split_sheet, valid_name, valid_sheet, Actor, AnswerType, Cell, CellKind, CellState,
    Cite, Decision, Format, Mark, Provenance, Suggestion, Usage,
};
use anyhow::{bail, Context, Result};
use notify::Watcher;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

pub struct Store {
    root: PathBuf,
}

/// `.recalc/config.toml`. Absent file ⇒ all defaults.
#[derive(Deserialize)]
#[serde(default)]
pub struct Config {
    pub shell: String,
    pub timeout_ms: u64,
    /// Global working-directory override for command cells.
    pub cwd: Option<String>,
    /// Extra environment variables for command cells.
    pub env: std::collections::HashMap<String, String>,
    /// Extra tokens for the danger scan, on top of the built-ins.
    pub danger_tokens: Vec<String>,
    /// `@agent(...)` command templates by name, adding to or overriding the
    /// builtins; `{task}`, `{mcp}` and `{recalc}` are substituted.
    pub agents: std::collections::HashMap<String, String>,
    /// Model aliases for `model=` on an agent call: `[models] fast = "claude-haiku-5-5"`.
    #[serde(default)]
    pub models: std::collections::HashMap<String, String>,
    /// How to continue a custom `[agents]` template's conversation, by name:
    /// `[resume] mine = "mine --resume {session}"`. Templates that run
    /// `claude` or `codex exec` don't need one.
    #[serde(default)]
    pub resume: std::collections::HashMap<String, String>,
    /// Executable paths for builtin agents not on PATH, e.g. `claude = "/opt/tools/claude"`.
    pub paths: std::collections::HashMap<String, String>,
    /// Timeout for `@agent(...)` runs.
    pub agent_timeout_ms: u64,
    /// Maximum concurrent `@agent(...)` runs; the rest queue.
    pub max_agents: usize,
    /// The driver agent: `claude`, `codex`, or a name under `[agents]`.
    pub driver: String,
    /// Dollar limit on the board's agent spend, after which queued agent
    /// cells wait for a person. None: no limit.
    pub budget_usd: Option<f64>,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            shell: "sh".into(),
            timeout_ms: 30_000,
            cwd: None,
            env: Default::default(),
            danger_tokens: Vec::new(),
            agents: Default::default(),
            models: Default::default(),
            resume: Default::default(),
            paths: Default::default(),
            agent_timeout_ms: 600_000,
            max_agents: 4,
            driver: "claude".into(),
            budget_usd: None,
        }
    }
}

/// The Driver's usage on a board: each conversation it has had, and each of
/// their runs that has ended, as Claude Code recorded it.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct DriverUsage {
    pub seen: Vec<String>,
    pub runs: Vec<crate::session::LastRun>,
}

/// The store's change stream: names of cells whose files changed on disk.
/// Consumers re-read the cell to see what changed.
pub struct Watch {
    pub rx: mpsc::UnboundedReceiver<String>,
    _watcher: notify::RecommendedWatcher,
}

/// On-disk shape of meta/<name>.toml.
#[derive(Serialize, Deserialize)]
struct Meta {
    kind: CellKind,
    state: CellState,
    #[serde(default)]
    deps: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    stderr: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    duration_ms: Option<u64>,
    #[serde(default)]
    confirmed: bool,
    #[serde(default)]
    series: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    def_by: Option<Actor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    decision: Option<Decision>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pending_suggestions: Vec<Suggestion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mark: Option<Mark>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    format: Option<Format>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cite: Option<Cite>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "shape")]
    answer_type: Option<AnswerType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    usage: Option<Usage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    provenance: Provenance,
}

impl Meta {
    fn of(cell: &Cell) -> Meta {
        Meta {
            kind: cell.kind,
            state: cell.state.clone(),
            deps: cell.deps.clone(),
            exit_code: cell.exit_code,
            stderr: cell.stderr.clone(),
            duration_ms: cell.duration_ms,
            confirmed: cell.confirmed,
            series: cell.series,
            def_by: cell.def_by.clone(),
            decision: cell.decision.clone(),
            pending_suggestions: cell.pending_suggestions.clone(),
            mark: cell.mark.clone(),
            format: cell.format.clone(),
            cite: cell.cite.clone(),
            answer_type: cell.answer_type.clone(),
            usage: cell.usage.clone(),
            session_id: cell.session_id.clone(),
            provenance: cell.provenance.clone(),
        }
    }
}

/// `.recalc/sheets.toml`: the first sheet's name and the others' order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sheets {
    /// Today's grid; its cells have no prefix.
    #[serde(default = "first_sheet")]
    pub first: String,
    /// The other sheets, in tab order.
    #[serde(default)]
    pub others: Vec<String>,
}

fn first_sheet() -> String {
    "Sheet1".into()
}

impl Default for Sheets {
    fn default() -> Self {
        Sheets {
            first: first_sheet(),
            others: Vec::new(),
        }
    }
}

impl Sheets {
    /// The sheet `name` names, whatever its capitals: sheet names are case-insensitive.
    pub fn find(&self, name: &str) -> Option<&str> {
        std::iter::once(&self.first)
            .chain(&self.others)
            .find(|s| s.eq_ignore_ascii_case(name))
            .map(String::as_str)
    }

    /// The prefix a sheet a reference names gives its cells: None for the
    /// first sheet (its cells are unprefixed), else the board's spelling,
    /// or the name as written for a sheet that doesn't exist yet.
    pub fn target<'a>(&'a self, written: &'a str) -> Option<&'a str> {
        if written.eq_ignore_ascii_case(&self.first) {
            None
        } else {
            Some(self.find(written).unwrap_or(written))
        }
    }
}

/// What in `.recalc/` belongs to this computer rather than the board: what's
/// running, requests in flight, and agent configs that name this computer's
/// paths. Everything else is the board, to commit or copy with its folder.
const GITIGNORE: &str = "\
# Written by recalc: what belongs to this computer, not the board.
engine.pid
running/
inbox/
answers/
mcp*.json
driver_session
.tmp-*
";

impl Store {
    /// Open (creating if absent) the `.recalc/` directory under `dir`.
    pub fn open(dir: &Path) -> Result<Store> {
        let root = dir.join(".recalc");
        for sub in ["cells", "defs", "meta", "history", "sheets"] {
            fs::create_dir_all(root.join(sub))
                .with_context(|| format!("creating {}", root.join(sub).display()))?;
        }
        // Left alone once there, so a person can change it.
        let ignore = root.join(".gitignore");
        if !ignore.exists() {
            write_atomic(&ignore, GITIGNORE.trim_end())?;
        }
        // A board just unzipped from an export: its outside documents' copies.
        crate::bundle::apply_export(dir)?;
        Ok(Store { root })
    }

    /// Claim the computing-engine role for this board; other engines mirror
    /// its results through the watch. Guarded by a pid file across processes
    /// and a process-local set within this one.
    pub fn try_claim_engine(&self) -> bool {
        use std::io::Write;
        let mut claimed = claimed_dirs().lock().expect("claim lock");
        if claimed.contains(&self.root) {
            return false;
        }
        let path = self.root.join("engine.pid");
        let mine = std::process::id().to_string();
        // O_EXCL create decides a race between two starts; a dead holder's
        // file is removed and the create retried once.
        for _ in 0..2 {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut f) => {
                    if f.write_all(mine.as_bytes()).is_err() {
                        return false;
                    }
                    claimed.insert(self.root.clone());
                    return true;
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    let holder = fs::read_to_string(&path).unwrap_or_default();
                    let dead = holder
                        .trim()
                        .parse::<i32>()
                        .map(|pid| {
                            pid != std::process::id() as i32 && unsafe { libc::kill(pid, 0) } != 0
                        })
                        .unwrap_or(true);
                    if !dead {
                        return false;
                    }
                    let _ = fs::remove_file(&path); // stale; retry the create
                }
                Err(_) => return false,
            }
        }
        false
    }

    /// Leave the answer to a running agent's question in `.recalc/answers/<name>`.
    pub fn write_answer(&self, name: &str, answer: &str) -> Result<()> {
        let dir = self.root.join("answers");
        fs::create_dir_all(&dir)?;
        write_atomic(&dir.join(name), answer)
    }

    /// Take an answer left for `name`, if there is one.
    pub fn take_answer(&self, name: &str) -> Option<String> {
        let path = self.root.join("answers").join(name);
        let raw = fs::read_to_string(&path).ok()?;
        let _ = fs::remove_file(&path);
        Some(raw)
    }

    /// Record this process's pid as owner of `name`'s run. Must be written
    /// before the cell goes Computing, so no reader sees an unowned run.
    pub fn mark_running(&self, name: &str) -> Result<()> {
        let dir = self.root.join("running");
        fs::create_dir_all(&dir)?;
        write_atomic(&dir.join(name), &std::process::id().to_string())
    }

    /// Hand `name`'s agent run to the active engine: a marker with no pid, so
    /// its orphan sweep picks the run up, carrying the run's nesting depth.
    pub fn mark_handoff(&self, name: &str, depth: u32) -> Result<()> {
        let dir = self.root.join("running");
        fs::create_dir_all(&dir)?;
        write_atomic(&dir.join(name), &format!("handoff {depth}"))
    }

    /// The depth a handed-off run must keep, if `name`'s run was handed off.
    pub fn handoff_depth(&self, name: &str) -> Option<u32> {
        let owner = fs::read_to_string(self.root.join("running").join(name)).ok()?;
        owner.trim().strip_prefix("handoff ")?.parse().ok()
    }

    /// Another process holds the computing-engine role and is alive.
    pub fn active_engine_elsewhere(&self) -> bool {
        let owner = fs::read_to_string(self.root.join("engine.pid")).unwrap_or_default();
        match owner.trim().parse::<i32>() {
            Ok(pid) => pid != std::process::id() as i32 && unsafe { libc::kill(pid, 0) } == 0,
            Err(_) => false,
        }
    }

    pub fn clear_running(&self, name: &str) {
        let _ = fs::remove_file(self.root.join("running").join(name));
    }

    /// `name`'s run has no owner on record or its owning process is dead.
    pub fn run_orphaned(&self, name: &str) -> bool {
        let owner = fs::read_to_string(self.root.join("running").join(name)).unwrap_or_default();
        match owner.trim().parse::<i32>() {
            Ok(pid) => pid != std::process::id() as i32 && unsafe { libc::kill(pid, 0) } != 0,
            Err(_) => true,
        }
    }

    /// This process owns `name`'s run in flight.
    pub fn owns_run(&self, name: &str) -> bool {
        let owner = fs::read_to_string(self.root.join("running").join(name)).unwrap_or_default();
        owner.trim() == std::process::id().to_string()
    }

    pub fn release_engine(&self) {
        claimed_dirs()
            .lock()
            .expect("claim lock")
            .remove(&self.root);
        let path = self.root.join("engine.pid");
        if let Ok(s) = fs::read_to_string(&path) {
            if s.trim() == std::process::id().to_string() {
                let _ = fs::remove_file(&path);
            }
        }
    }

    /// The frontend's serialized view layout, opaque to the store; empty when never saved.
    pub fn layout(&self) -> Result<String> {
        match fs::read_to_string(self.root.join("layout.json")) {
            Ok(s) => Ok(s),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn set_layout(&self, json: &str) -> Result<()> {
        write_atomic(&self.root.join("layout.json"), json)
    }

    /// Read `.recalc/config.toml`; all defaults when the file is absent.
    pub fn config(&self) -> Result<Config> {
        match fs::read_to_string(self.root.join("config.toml")) {
            Ok(text) => toml::from_str(&text).context("config.toml"),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e.into()),
        }
    }

    /// When `config.toml` last changed, and its size: what a reload compares.
    /// None while there's no file.
    pub fn config_stamp(&self) -> Option<(std::time::SystemTime, u64)> {
        let meta = fs::metadata(self.root.join("config.toml")).ok()?;
        Some((meta.modified().ok()?, meta.len()))
    }

    /// Set `driver = "<agent>"` in `config.toml`, preserving the rest of the file.
    pub fn set_driver(&self, agent: &str) -> Result<()> {
        self.set_top_key("driver", Some(toml::Value::String(agent.to_string())))
    }

    /// Set or clear `budget_usd` in `config.toml`, preserving the rest of the file.
    pub fn set_budget(&self, dollars: Option<f64>) -> Result<()> {
        self.set_top_key("budget_usd", dollars.map(toml::Value::Float))
    }

    /// Write `key = value` as a top-level key (before any `[table]`), or remove
    /// it for None, leaving other lines and comments untouched.
    fn set_top_key(&self, key: &str, value: Option<toml::Value>) -> Result<()> {
        let path = self.root.join("config.toml");
        let text = fs::read_to_string(&path).unwrap_or_default();
        let line = value.map(|v| format!("{key} = {v}"));
        let mut out: Vec<String> = Vec::new();
        let mut done = false;
        let mut in_table = false;
        for l in text.lines() {
            let t = l.trim_start();
            in_table |= t.starts_with('[');
            let is_key = !in_table
                && t.strip_prefix(key)
                    .is_some_and(|rest| rest.trim_start().starts_with('='));
            if is_key && !done {
                out.extend(line.clone());
                done = true;
            } else if !is_key {
                out.push(l.to_string());
            }
        }
        if !done {
            if let Some(line) = line {
                out.insert(0, line);
            }
        }
        write_atomic(&path, out.join("\n").trim_end())
    }

    /// Where the board's agent MCP config lives (see `ensure_mcp_config`).
    pub fn mcp_config_path(&self) -> PathBuf {
        self.root.join("mcp.json")
    }

    /// Write `.recalc/mcp.json` pointing an agent at `recalc mcp` and return
    /// its path. Headless `@` runs use `ensure_mcp_config_for_cell` instead.
    pub fn ensure_mcp_config(&self) -> Result<PathBuf> {
        self.write_mcp_config(self.mcp_config_path(), &["mcp"])
    }

    /// `.recalc/mcp-<id>.json`, for a client whose writes carry a fixed id.
    pub fn ensure_mcp_config_for(&self, id: &str) -> Result<PathBuf> {
        check_name(id)?;
        self.write_mcp_config(
            self.root.join(format!("mcp-{id}.json")),
            &["mcp", "--id", id],
        )
    }

    /// `.recalc/mcp-cell-<name>.json`, for a headless run of agent cell `name`:
    /// writes carry the cell's name and shell access through the grid is off.
    pub fn ensure_mcp_config_for_cell(&self, name: &str, depth: u32) -> Result<PathBuf> {
        check_name(name)?;
        let depth = depth.to_string();
        self.write_mcp_config(
            self.mcp_cell_config_path(name),
            &["mcp", "--cell", name, "--depth", &depth],
        )
    }

    pub fn mcp_cell_config_path(&self, name: &str) -> PathBuf {
        self.root.join(format!("mcp-cell-{name}.json"))
    }

    /// Atomic, so an agent starting mid-write never reads a partial config.
    fn write_mcp_config(&self, path: PathBuf, args: &[&str]) -> Result<PathBuf> {
        let recalc = std::env::current_exe().context("locating recalc binary")?;
        let config = serde_json::json!({
            "mcpServers": { "recalc": { "command": recalc, "args": args } }
        });
        write_atomic(&path, &config.to_string())?;
        Ok(path)
    }

    /// Write `.recalc/prompts/<name>.md` if it differs from `text`, and return its path.
    /// A board's own version of a prompt, `.recalc/prompts/<name>.local.md`,
    /// used in place of the built-in one when present and not empty.
    pub fn prompt_override(&self, name: &str) -> Option<String> {
        let text =
            fs::read_to_string(self.root.join("prompts").join(format!("{name}.local.md"))).ok()?;
        (!text.trim().is_empty()).then_some(text)
    }

    pub fn ensure_prompt_file(&self, name: &str, text: &str) -> Result<PathBuf> {
        let dir = self.root.join("prompts");
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{name}.md"));
        if fs::read_to_string(&path).ok().as_deref() != Some(text) {
            fs::write(&path, text)?;
        }
        Ok(path)
    }

    /// The driver's conversation id for this board, if one was minted.
    pub fn driver_session(&self) -> Option<String> {
        read_entry(&self.root.join("driver_session")).ok().flatten()
    }

    pub fn set_driver_session(&self, id: &str) -> Result<()> {
        write_atomic(&self.root.join("driver_session"), id)
    }

    /// What the Driver's finished runs used, as `.recalc/driver_usage.json`.
    pub fn driver_usage(&self) -> DriverUsage {
        fs::read_to_string(self.root.join("driver_usage.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn set_driver_usage(&self, usage: &DriverUsage) -> Result<()> {
        write_atomic(
            &self.root.join("driver_usage.json"),
            &serde_json::to_string_pretty(usage)?,
        )
    }

    pub fn clear_driver_session(&self) -> Result<()> {
        match fs::remove_file(self.root.join("driver_session")) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }

    /// Queue a request for the board's window (open a session, select a cell)
    /// as a file in `.recalc/inbox/`, the only channel from an MCP client to
    /// the window. Requests wait until a window takes them.
    pub fn request(&self, kind: &str, name: &str) -> Result<()> {
        check_name(name)?;
        let dir = self.root.join("inbox");
        fs::create_dir_all(&dir)?;
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros())
            .unwrap_or(0);
        write_atomic(&dir.join(format!("{at:020}-{kind}-{name}")), "")
    }

    /// Take every pending request, oldest first, as `(kind, name)`.
    pub fn take_requests(&self) -> Vec<(String, String)> {
        let dir = self.root.join("inbox");
        let Ok(entries) = fs::read_dir(&dir) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| !n.starts_with('.'))
            .collect();
        names.sort();
        let mut out = Vec::new();
        for file in names {
            let _ = fs::remove_file(dir.join(&file));
            // <micros>-<kind>-<name>; a kind has no dash, so the rest is the
            // name, whatever a sheet's name holds.
            let mut parts = file.splitn(3, '-');
            let (Some(_), Some(kind), Some(name)) = (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            out.push((kind.to_string(), name.to_string()));
        }
        out
    }

    /// The folder of `kind` (cells, defs, meta, history) that holds `name`,
    /// and the name's file there: a cell on another sheet lives in that
    /// sheet's own folders, `sheets/Notes/cells/A2`.
    fn place<'a>(&self, kind: &str, name: &'a str) -> (PathBuf, &'a str) {
        match split_sheet(name) {
            (Some(sheet), local) => (self.root.join("sheets").join(sheet).join(kind), local),
            (None, local) => (self.root.join(kind), local),
        }
    }

    /// The backing file for a cell's value, which `{name:file}` resolves to.
    pub fn value_path(&self, name: &str) -> PathBuf {
        let (dir, file) = self.place("cells", name);
        dir.join(file)
    }

    fn def_path(&self, name: &str) -> PathBuf {
        let (dir, file) = self.place("defs", name);
        dir.join(file)
    }

    fn meta_path(&self, name: &str) -> PathBuf {
        let (dir, file) = self.place("meta", name);
        dir.join(format!("{file}.toml"))
    }

    fn history_path(&self, name: &str) -> PathBuf {
        let (dir, file) = self.place("history", name);
        dir.join(format!("{file}.log"))
    }

    /// Before writing a cell on another sheet: its folders, and the sheet in
    /// `sheets.toml` if it's new (a write is how an agent makes a sheet).
    fn ensure_sheet_of(&self, name: &str) -> Result<()> {
        let (Some(sheet), _) = split_sheet(name) else {
            return Ok(());
        };
        if self.root.join("sheets").join(sheet).join("cells").is_dir() {
            return Ok(());
        }
        self.add_sheet(sheet)
    }

    /// The board's sheets: the first (today's grid, its cells unprefixed)
    /// by its name, then the others in order. A sheet folder that
    /// `sheets.toml` doesn't list yet (another writer's) comes last.
    pub fn sheets(&self) -> Sheets {
        let mut sheets: Sheets = fs::read_to_string(self.root.join("sheets.toml"))
            .ok()
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default();
        sheets
            .others
            .retain(|s| self.root.join("sheets").join(s).is_dir());
        let mut found: Vec<String> = fs::read_dir(self.root.join("sheets"))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|s| valid_sheet(s) && !sheets.others.contains(s))
            .collect();
        found.sort();
        sheets.others.extend(found);
        sheets
    }

    /// A cell however someone wrote it (`'My Notes'!A2`, `notes!A2`,
    /// `Sheet1!A2`) as recalc names it; text that isn't a reference comes
    /// back trimmed. The engine's `canonical` does the same from its cache.
    pub fn canonical(&self, text: &str) -> String {
        let Some(name) = crate::model::ref_name(text) else {
            return text.trim().to_string();
        };
        match split_sheet(&name) {
            (Some(sheet), local) => on_sheet(self.sheets().target(sheet), local),
            (None, _) => name,
        }
    }

    fn write_sheets(&self, sheets: &Sheets) -> Result<()> {
        write_atomic(
            &self.root.join("sheets.toml"),
            toml::to_string(sheets)?.trim_end(),
        )
    }

    /// Add an empty sheet at the end. Its name must be free, whatever its capitals.
    pub fn add_sheet(&self, name: &str) -> Result<()> {
        let mut sheets = self.sheets();
        if !valid_sheet(name) {
            bail!("{name:?} can't be a sheet name: up to 31 characters, none of / \\ ? * : [ ] ! {{ }} \" , ;, not starting with # or @, and no apostrophe or space at either end");
        }
        if let Some(taken) = sheets.find(name) {
            bail!("there's already a sheet named {taken}");
        }
        for kind in ["cells", "defs", "meta", "history"] {
            fs::create_dir_all(self.root.join("sheets").join(name).join(kind))?;
        }
        sheets.others.push(name.to_string());
        self.write_sheets(&sheets)
    }

    /// Rename a sheet; its cells move with its folder. The engine rewrites
    /// references to it.
    pub fn rename_sheet(&self, from: &str, to: &str) -> Result<()> {
        let mut sheets = self.sheets();
        if !valid_sheet(to) {
            bail!("{to:?} can't be a sheet name");
        }
        if sheets
            .find(to)
            .is_some_and(|t| !t.eq_ignore_ascii_case(from))
        {
            bail!("there's already a sheet named {to}");
        }
        if sheets.first == from {
            sheets.first = to.to_string();
            return self.write_sheets(&sheets);
        }
        let Some(at) = sheets.others.iter().position(|s| s == from) else {
            bail!("no sheet named {from}");
        };
        fs::rename(
            self.root.join("sheets").join(from),
            self.root.join("sheets").join(to),
        )?;
        sheets.others[at] = to.to_string();
        self.write_sheets(&sheets)
    }

    /// Delete a sheet and its cells. The first sheet stays.
    pub fn delete_sheet(&self, name: &str) -> Result<()> {
        let mut sheets = self.sheets();
        if sheets.first == name {
            bail!("the first sheet can't be deleted");
        }
        let Some(at) = sheets.others.iter().position(|s| s == name) else {
            bail!("no sheet named {name}");
        };
        fs::remove_dir_all(self.root.join("sheets").join(name))?;
        sheets.others.remove(at);
        self.write_sheets(&sheets)
    }

    /// Put sheet `name` at `index` among the others (the first stays first).
    pub fn move_sheet(&self, name: &str, index: usize) -> Result<()> {
        let mut sheets = self.sheets();
        let Some(at) = sheets.others.iter().position(|s| s == name) else {
            bail!("no sheet named {name}");
        };
        let sheet = sheets.others.remove(at);
        sheets.others.insert(index.min(sheets.others.len()), sheet);
        self.write_sheets(&sheets)
    }

    pub fn get(&self, name: &str) -> Result<Option<Cell>> {
        let value = read_entry(&self.value_path(name))?;
        let source = read_entry(&self.def_path(name))?;
        if value.is_none() && source.is_none() {
            return Ok(None);
        }
        let meta = self.read_meta(name)?.unwrap_or_else(|| {
            // No meta: the file was written from outside recalc.
            let mut cell = Cell::new(name);
            cell.kind = source
                .as_deref()
                .map_or(CellKind::Literal, CellKind::of_source);
            Meta::of(&cell)
        });
        // A cell with no formula on disk is a value, whatever its meta says:
        // another writer turning a formula into a value deletes the formula
        // before it rewrites the meta, and a reader can land in between.
        let kind = if source.is_none() {
            CellKind::Literal
        } else {
            meta.kind
        };
        Ok(Some(Cell {
            name: name.to_string(),
            kind,
            source,
            value: value.unwrap_or_default(),
            deps: meta.deps,
            state: meta.state,
            exit_code: meta.exit_code,
            stderr: meta.stderr,
            duration_ms: meta.duration_ms,
            confirmed: meta.confirmed,
            series: meta.series,
            def_by: meta.def_by,
            decision: meta.decision,
            pending_suggestions: meta.pending_suggestions,
            mark: meta.mark,
            format: meta.format,
            cite: meta.cite,
            answer_type: meta.answer_type,
            usage: meta.usage,
            session_id: meta.session_id,
            activity: None,
            provenance: meta.provenance,
        }))
    }

    /// Write a literal value, dropping any source.
    pub fn set_value(&self, name: &str, value: &str, by: Actor) -> Result<()> {
        check_name(name)?;
        self.ensure_sheet_of(name)?;
        self.append_history(name, value)?;
        write_atomic(&self.value_path(name), value)?;
        let def = self.def_path(name);
        if def.exists() {
            fs::remove_file(&def)?;
        }
        let mut cell = Cell::new(name);
        cell.provenance = Provenance::now(by);
        self.write_meta(name, &Meta::of(&cell))
    }

    /// Install a source with its parsed deps; the engine computes the value later.
    pub fn set_def(&self, name: &str, source: &str, deps: &[String], by: Actor) -> Result<()> {
        check_name(name)?;
        self.ensure_sheet_of(name)?;
        write_atomic(&self.def_path(name), source)?;
        let mut cell = Cell::new(name);
        cell.kind = CellKind::of_source(source);
        cell.state = CellState::Stale;
        cell.deps = deps.to_vec();
        cell.def_by = Some(by.clone());
        cell.provenance = Provenance::now(by);
        self.write_meta(name, &Meta::of(&cell))
    }

    /// Watch `cells/`, `defs/` and `meta/` for changes from any writer. Meta is
    /// watched too, since marks and citations change no value. Not debounced.
    pub fn watch(&self) -> Result<Watch> {
        let (tx, rx) = mpsc::unbounded_channel();
        // Events carry canonical paths (macOS /var -> /private/var).
        let cells_dir = self.root.join("cells").canonicalize()?;
        let defs_dir = self.root.join("defs").canonicalize()?;
        let meta_dir = self.root.join("meta").canonicalize()?;
        let sheets_dir = self.root.join("sheets").canonicalize()?;
        let (c, d, m) = (cells_dir.clone(), defs_dir.clone(), meta_dir.clone());
        let sh = sheets_dir.clone();
        let mut watcher =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                let Ok(event) = res else { return };
                // Linux reports opening and reading a file. Reading a changed cell
                // would report it changed again, round and round.
                if matches!(event.kind, notify::EventKind::Access(_)) {
                    return;
                }
                for path in event.paths {
                    let Some(file) = path.file_name().and_then(|n| n.to_str()) else {
                        continue;
                    };
                    // A sheet added, renamed or deleted (its folder, or
                    // sheets.toml): "" asks the engine to look again.
                    if path.parent() == Some(sh.as_path())
                        || (path.parent() == sh.parent() && file == "sheets.toml")
                    {
                        let _ = tx.send(String::new());
                        continue;
                    }
                    // Another sheet's file: sheets/<sheet>/<kind>/<file>.
                    let (sheet, kind) = match path.parent().and_then(|p| p.strip_prefix(&sh).ok()) {
                        Some(rel) => {
                            let mut parts =
                                rel.components().map(|c| c.as_os_str().to_string_lossy());
                            match (parts.next(), parts.next(), parts.next()) {
                                (Some(sheet), Some(kind), None) => {
                                    (Some(sheet.into_owned()), kind.into_owned())
                                }
                                _ => continue,
                            }
                        }
                        None if path.parent() == Some(m.as_path()) => (None, "meta".into()),
                        None if path.parent() == Some(c.as_path()) => (None, "cells".into()),
                        None if path.parent() == Some(d.as_path()) => (None, "defs".into()),
                        None => continue,
                    };
                    let local = match kind.as_str() {
                        "meta" => match file.strip_suffix(".toml") {
                            Some(n) => n,
                            None => continue,
                        },
                        "cells" | "defs" => file,
                        _ => continue,
                    };
                    let name = on_sheet(sheet.as_deref(), local);
                    if !valid_name(&name) {
                        continue; // .tmp-* staging files and other noise
                    }
                    let _ = tx.send(name);
                }
            })?;
        watcher.watch(&cells_dir, notify::RecursiveMode::NonRecursive)?;
        watcher.watch(&defs_dir, notify::RecursiveMode::NonRecursive)?;
        watcher.watch(&meta_dir, notify::RecursiveMode::NonRecursive)?;
        watcher.watch(&sheets_dir, notify::RecursiveMode::Recursive)?;
        // For sheets.toml; only direct children of .recalc/ report here.
        watcher.watch(
            &self.root.canonicalize()?,
            notify::RecursiveMode::NonRecursive,
        )?;
        Ok(Watch {
            rx,
            _watcher: watcher,
        })
    }

    /// Remove a cell's value, def and meta; history is kept.
    pub fn delete(&self, name: &str) -> Result<()> {
        check_name(name)?;
        for path in [
            self.value_path(name),
            self.def_path(name),
            self.meta_path(name),
        ] {
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    /// Write a cell's value, def and meta under `name`, e.g. at a new address after a move.
    pub fn put(&self, name: &str, cell: &Cell) -> Result<()> {
        check_name(name)?;
        self.ensure_sheet_of(name)?;
        write_atomic(&self.value_path(name), &cell.value)?;
        match &cell.source {
            Some(src) => write_atomic(&self.def_path(name), src)?,
            None => match fs::remove_file(self.def_path(name)) {
                Ok(()) | Err(_) => {}
            },
        }
        self.write_meta(name, &Meta::of(cell))
    }

    pub fn delete_many(&self, names: &[String]) -> Result<()> {
        each(names, |n| self.delete(n))
    }

    /// Write cells at their addresses, given what each address held `before`;
    /// only files whose contents differ are written.
    pub fn put_many(&self, cells: &[(&Cell, Option<&Cell>)]) -> Result<()> {
        each(cells, |(c, before)| self.put_over(c, *before))
    }

    fn put_over(&self, cell: &Cell, before: Option<&Cell>) -> Result<()> {
        let name = &cell.name;
        check_name(name)?;
        self.ensure_sheet_of(name)?;
        if before.map(|b| &b.value) != Some(&cell.value) {
            write_atomic(&self.value_path(name), &cell.value)?;
        }
        if before.map(|b| &b.source) != Some(&cell.source) {
            match &cell.source {
                Some(src) => write_atomic(&self.def_path(name), src)?,
                None => match fs::remove_file(self.def_path(name)) {
                    Ok(()) | Err(_) => {}
                },
            }
        }
        let meta = toml::to_string(&Meta::of(cell))?;
        let same = before.is_some_and(|b| toml::to_string(&Meta::of(b)).is_ok_and(|m| m == meta));
        if !same {
            write_atomic(&self.meta_path(name), &meta)?;
        }
        Ok(())
    }

    /// Move history logs with their cells, `(from, to)` each. Two passes via
    /// staging names, since a move's target may be another move's source.
    pub fn move_histories(&self, moves: &[(String, String)]) -> Result<()> {
        let staged = |name: &str| {
            let (dir, file) = self.place("history", name);
            dir.join(format!(".mv-{}-{file}.log", std::process::id()))
        };
        let present: Vec<&(String, String)> = moves
            .iter()
            .filter(|(from, _)| self.history_path(from).exists())
            .collect();
        each(&present, |(from, _)| {
            fs::rename(self.history_path(from), staged(from)).map_err(Into::into)
        })?;
        each(&present, |(from, to)| {
            self.ensure_sheet_of(to)?;
            fs::rename(staged(from), self.history_path(to)).map_err(Into::into)
        })
    }

    /// Forget a deleted cell's history so a later cell at its address doesn't inherit it.
    pub fn delete_history(&self, name: &str) -> Result<()> {
        match fs::remove_file(self.history_path(name)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    /// Persist a computed value and meta, leaving the def alone.
    pub fn update(&self, cell: &Cell) -> Result<()> {
        self.append_history(&cell.name, &cell.value)?;
        write_atomic(&self.value_path(&cell.name), &cell.value)?;
        self.write_meta(&cell.name, &Meta::of(cell))
    }

    /// On value change, append an escaped `timestamp\tvalue` line to
    /// `history/<name>.log`, trimming to the last 500 lines past 256 KiB.
    fn append_history(&self, name: &str, value: &str) -> Result<()> {
        let unchanged = read_entry(&self.value_path(name))?.as_deref() == Some(value);
        if unchanged {
            return Ok(());
        }
        self.ensure_sheet_of(name)?;
        let path = self.history_path(name);
        let escaped = value
            .replace('\\', r"\\")
            .replace('\n', r"\n")
            .replace('\t', r"\t");
        let line = format!("{}\t{escaped}\n", Provenance::now(Actor::Engine).at);
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        file.write_all(line.as_bytes())?;
        if file.metadata()?.len() > 256 * 1024 {
            let all = fs::read_to_string(&path)?;
            let lines: Vec<&str> = all.lines().collect();
            let keep = &lines[lines.len().saturating_sub(500)..];
            write_atomic(&path, &(keep.join("\n") + "\n"))?;
        }
        Ok(())
    }

    /// The last `limit` history entries for a cell, oldest first.
    pub fn history(&self, name: &str, limit: usize) -> Result<Vec<(u64, String)>> {
        let path = self.history_path(name);
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let lines: Vec<&str> = text.lines().collect();
        Ok(lines[lines.len().saturating_sub(limit)..]
            .iter()
            .filter_map(|l| {
                let (ts, v) = l.split_once('\t')?;
                Some((ts.parse().ok()?, unescape(v)))
            })
            .collect())
    }

    /// Persist only the meta (state) of a cell.
    pub fn set_state(&self, cell: &Cell) -> Result<()> {
        self.write_meta(&cell.name, &Meta::of(cell))
    }

    pub fn list(&self) -> Result<Vec<Cell>> {
        let mut names = std::collections::BTreeSet::new();
        let sheets = self.sheets();
        let homes = std::iter::once((None, self.root.clone())).chain(
            sheets
                .others
                .iter()
                .map(|s| (Some(s.as_str()), self.root.join("sheets").join(s))),
        );
        for (sheet, home) in homes {
            for sub in ["cells", "defs"] {
                let Ok(entries) = fs::read_dir(home.join(sub)) else {
                    continue;
                };
                for entry in entries {
                    let file = entry?.file_name().to_string_lossy().into_owned();
                    let name = on_sheet(sheet, &file);
                    if valid_name(&name) {
                        names.insert(name);
                    }
                }
            }
        }
        let mut cells = Vec::with_capacity(names.len());
        for name in &names {
            if let Some(cell) = self.get(name)? {
                cells.push(cell);
            }
        }
        Ok(cells)
    }

    fn read_meta(&self, name: &str) -> Result<Option<Meta>> {
        match fs::read_to_string(self.meta_path(name)) {
            Ok(s) => Ok(Some(
                toml::from_str(&s).with_context(|| format!("meta for {name}"))?,
            )),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn write_meta(&self, name: &str, meta: &Meta) -> Result<()> {
        self.ensure_sheet_of(name)?;
        write_atomic(&self.meta_path(name), &toml::to_string(meta)?)
    }
}

/// `f` over `items`, stopping at the first error. Sequential on purpose:
/// parallel writes into one directory measured slower.
fn each<T>(items: &[T], f: impl Fn(&T) -> Result<()>) -> Result<()> {
    items.iter().try_for_each(f)
}

fn claimed_dirs() -> &'static std::sync::Mutex<std::collections::HashSet<PathBuf>> {
    static CLAIMED: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<PathBuf>>> =
        std::sync::OnceLock::new();
    CLAIMED.get_or_init(Default::default)
}

/// Inverse of the history-line escaping. A replace chain would mangle a
/// literal backslash followed by `n`.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

pub(crate) fn check_name(name: &str) -> Result<()> {
    if !crate::model::valid_name(name) {
        bail!("invalid cell name {name:?}: a name like total or B2, or a cell on a sheet like Notes!B2");
    }
    Ok(())
}

/// Read a value or def file, minus its one trailing newline; None if absent.
fn read_entry(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(mut s) => {
            if s.ends_with('\n') {
                s.pop();
            }
            Ok(Some(s))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(anyhow::Error::from(e).context(format!("reading {}", path.display()))),
    }
}

/// Write to a temp file and rename, so readers never see partial content. The
/// pid in the temp name keeps concurrent processes off each other's temp file.
fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    let dir = path.parent().expect("store paths always have a parent");
    let file_name = path
        .file_name()
        .expect("store paths always have a file name");
    let tmp = dir.join(format!(
        ".tmp-{}-{}",
        std::process::id(),
        file_name.to_string_lossy()
    ));
    fs::write(&tmp, format!("{contents}\n"))
        .with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("renaming to {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        (dir, store)
    }

    #[test]
    fn a_board_saved_with_the_old_shape_key_still_loads() {
        let (_d, st) = store();
        let mut cell = crate::model::Cell::new("B2");
        cell.value = "Yes".into();
        cell.answer_type = Some(crate::model::AnswerType::new(
            crate::model::AnswerKind::YesNo,
            &[],
        ));
        st.update(&cell).unwrap();
        // As a board saved before the rename has it.
        let meta = std::fs::read_to_string(st.meta_path("B2")).unwrap();
        assert!(meta.contains("[answer_type]"), "{meta}");
        std::fs::write(st.meta_path("B2"), meta.replace("[answer_type]", "[shape]")).unwrap();
        let loaded = st.get("B2").unwrap().unwrap();
        let t = loaded
            .answer_type
            .as_ref()
            .expect("the old [shape] section is read");
        assert_eq!(t.kind, crate::model::AnswerKind::YesNo);
        st.update(&loaded).unwrap();
        let text = std::fs::read_to_string(st.meta_path("B2")).unwrap();
        assert!(
            text.contains("[answer_type]") && !text.contains("[shape]"),
            "{text}"
        );
    }

    #[test]
    fn set_and_get_literal() {
        let (dir, store) = store();
        store.set_value("A1", "42", Actor::Cli).unwrap();
        let cell = store.get("A1").unwrap().unwrap();
        assert_eq!(cell.value, "42");
        assert_eq!(cell.kind, CellKind::Literal);
        assert_eq!(cell.provenance.by, Actor::Cli);
        // The value is the file's content.
        let raw = fs::read_to_string(dir.path().join(".recalc/cells/A1")).unwrap();
        assert_eq!(raw, "42\n");
    }

    #[test]
    fn multiline_value_round_trips() {
        let (_dir, store) = store();
        let v = "line one\nline two\n"; // trailing newline is part of the value
        store.set_value("out", v, Actor::Human).unwrap();
        assert_eq!(store.get("out").unwrap().unwrap().value, v);
    }

    #[test]
    fn set_def_records_kind() {
        let (_dir, store) = store();
        let deps = ["a".to_string(), "b".to_string()];
        store
            .set_def("total", "= {a} + {b}", &deps, Actor::Agent("a1".into()))
            .unwrap();
        let cell = store.get("total").unwrap().unwrap();
        assert_eq!(cell.kind, CellKind::Expr);
        assert_eq!(cell.source.as_deref(), Some("= {a} + {b}"));
        assert_eq!(cell.deps, deps);
        assert_eq!(cell.provenance.by, Actor::Agent("a1".into()));
    }

    #[test]
    fn literal_write_drops_def() {
        let (_dir, store) = store();
        store.set_def("x", "= 1 + 1", &[], Actor::Cli).unwrap();
        store.set_value("x", "pinned", Actor::Human).unwrap();
        let cell = store.get("x").unwrap().unwrap();
        assert_eq!(cell.kind, CellKind::Literal);
        assert_eq!(cell.source, None);
        assert_eq!(cell.value, "pinned");
    }

    #[test]
    fn external_bare_file_is_a_cell() {
        let (dir, store) = store();
        fs::write(dir.path().join(".recalc/cells/status"), "green\n").unwrap();
        let cell = store.get("status").unwrap().unwrap();
        assert_eq!(cell.value, "green");
        assert_eq!(cell.kind, CellKind::Literal);
    }

    #[test]
    fn delete_removes_every_file() {
        let (dir, store) = store();
        store.set_def("x", "= 1", &[], Actor::Cli).unwrap();
        store.delete("x").unwrap();
        assert!(store.get("x").unwrap().is_none());
        assert!(!dir.path().join(".recalc/meta/x.toml").exists());
        store.delete("never").unwrap(); // absent is fine
    }

    #[test]
    fn missing_cell_is_none() {
        let (_dir, store) = store();
        assert!(store.get("nope").unwrap().is_none());
    }

    #[test]
    fn list_unions_cells_and_defs() {
        let (_dir, store) = store();
        store.set_value("b", "2", Actor::Cli).unwrap();
        store.set_def("a", "$ echo hi", &[], Actor::Cli).unwrap();
        let names: Vec<_> = store.list().unwrap().into_iter().map(|c| c.name).collect();
        assert_eq!(names, ["a", "b"]);
    }

    #[test]
    fn a_board_keeps_what_belongs_to_this_computer_out_of_git() {
        let (dir, store) = store();
        let ignore = dir.path().join(".recalc/.gitignore");
        let text = fs::read_to_string(&ignore).unwrap();
        for line in [
            "engine.pid",
            "running/",
            "inbox/",
            "mcp*.json",
            "driver_session",
        ] {
            assert!(text.lines().any(|l| l == line), "{line} in {text}");
        }
        assert!(!text
            .lines()
            .any(|l| ["cells/", "defs/", "meta/", "history/"].contains(&l)));
        // A person's own version stays.
        fs::write(&ignore, "mine\n").unwrap();
        drop(store);
        Store::open(dir.path()).unwrap();
        assert_eq!(fs::read_to_string(&ignore).unwrap(), "mine\n");
    }

    #[test]
    fn a_cell_on_another_sheet_lives_in_that_sheets_folder() {
        let (dir, store) = store();
        store
            .set_value("Notes!A2", "the rules", Actor::Cli)
            .unwrap();
        store.set_value("A2", "main", Actor::Cli).unwrap();
        let file = dir.path().join(".recalc/sheets/Notes/cells/A2");
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            "the rules\n",
            "cat still works"
        );
        assert_eq!(store.get("Notes!A2").unwrap().unwrap().value, "the rules");
        assert_eq!(store.get("A2").unwrap().unwrap().value, "main");
        let names: Vec<String> = store.list().unwrap().into_iter().map(|c| c.name).collect();
        assert_eq!(names, ["A2", "Notes!A2"]);
        assert_eq!(
            store.sheets(),
            Sheets {
                first: "Sheet1".into(),
                others: vec!["Notes".into()]
            }
        );
        store
            .set_value("Notes!A2", "the new rules", Actor::Cli)
            .unwrap();
        assert_eq!(store.history("Notes!A2", 5).unwrap().len(), 2);

        // Sheets: added, found whatever the capitals, renamed, moved, deleted.
        store.add_sheet("My Notes").unwrap();
        assert!(store
            .add_sheet("notes")
            .unwrap_err()
            .to_string()
            .contains("already a sheet named Notes"));
        assert!(store.add_sheet("a:b").is_err());
        assert_eq!(store.sheets().find("my notes"), Some("My Notes"));
        store.rename_sheet("Notes", "Rules").unwrap();
        assert_eq!(
            store.get("Rules!A2").unwrap().unwrap().value,
            "the new rules"
        );
        assert_eq!(
            store.history("Rules!A2", 5).unwrap().len(),
            2,
            "history moves with the sheet"
        );
        store.move_sheet("My Notes", 1).unwrap();
        assert_eq!(store.sheets().others, ["Rules", "My Notes"]);
        store.rename_sheet("Sheet1", "Board").unwrap();
        assert_eq!(store.sheets().first, "Board");
        assert!(
            store.delete_sheet("Board").is_err(),
            "the first sheet stays"
        );
        store.delete_sheet("Rules").unwrap();
        assert!(store.get("Rules!A2").unwrap().is_none());
        assert_eq!(store.sheets().others, ["My Notes"]);
    }

    #[tokio::test]
    async fn the_watch_names_cells_on_other_sheets() {
        let (dir, store) = store();
        store.add_sheet("Notes").unwrap();
        let mut watch = store.watch().unwrap();
        fs::write(
            dir.path().join(".recalc/sheets/Notes/cells/B3"),
            "from outside\n",
        )
        .unwrap();
        let got = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let name = watch.rx.recv().await.unwrap();
                if name == "Notes!B3" {
                    return name;
                }
            }
        })
        .await
        .expect("the watch reports Notes!B3");
        assert_eq!(got, "Notes!B3");
    }

    #[test]
    fn invalid_name_rejected() {
        let (_dir, store) = store();
        assert!(store.set_value("../evil", "x", Actor::Cli).is_err());
        assert!(store.set_value("a b", "x", Actor::Cli).is_err());
    }

    #[test]
    fn session_id_rides_in_meta() {
        let (_dir, store) = store();
        store.set_def("b", "@claude('x')", &[], Actor::Cli).unwrap();
        let mut cell = store.get("b").unwrap().unwrap();
        assert_eq!(cell.session_id, None);
        cell.session_id = Some("1111-2222".into());
        store.update(&cell).unwrap();
        assert_eq!(
            store.get("b").unwrap().unwrap().session_id.as_deref(),
            Some("1111-2222")
        );
    }

    #[test]
    fn requests_queue_in_the_inbox_oldest_first() {
        let (_dir, store) = store();
        assert!(store.take_requests().is_empty());
        store.request("open_session", "B2").unwrap();
        store.request("show_cell", "plan_3").unwrap();
        assert!(
            store.request("show_cell", "../x").is_err(),
            "a request names a cell"
        );
        assert_eq!(
            store.take_requests(),
            [
                ("open_session".to_string(), "B2".to_string()),
                ("show_cell".to_string(), "plan_3".to_string())
            ]
        );
        assert!(store.take_requests().is_empty(), "taken means gone");
    }

    #[test]
    fn an_mcp_config_can_carry_an_id_and_the_driver_keeps_its_session() {
        let (_dir, store) = store();
        let path = store.ensure_mcp_config_for("driver").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("\"args\":[\"mcp\",\"--id\",\"driver\"]"),
            "{text}"
        );
        assert!(store.ensure_mcp_config_for("bad id").is_err());
        assert_eq!(store.driver_session(), None);
        store.set_driver_session("abc").unwrap();
        assert_eq!(store.driver_session().as_deref(), Some("abc"));
        let prompt = store.ensure_prompt_file("driver", "role text").unwrap();
        assert_eq!(std::fs::read_to_string(prompt).unwrap(), "role text");
    }

    #[test]
    fn rotation_keeps_entries_parseable() {
        let (_dir, store) = store();
        // Long values so a few hundred appends cross the 256 KiB threshold.
        let filler = "x".repeat(600);
        for i in 0..600 {
            store
                .set_value("n", &format!("{i} {filler}"), Actor::Cli)
                .unwrap();
        }
        let history = store.history("n", 600).unwrap();
        assert!(history.len() < 600, "rotation never triggered");
        // Every surviving entry is intact, with no merged lines at the rotation seam.
        for (j, (_, v)) in history.iter().rev().enumerate() {
            assert_eq!(*v, format!("{} {filler}", 599 - j));
        }
    }

    #[test]
    fn setting_the_driver_keeps_the_rest_of_the_config() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        store.set_driver("codex").unwrap();
        assert_eq!(store.config().unwrap().driver, "codex");
        let path = dir.path().join(".recalc/config.toml");
        fs::write(
            &path,
            "# my settings\nmax_agents = 2\ndriver = \"claude\"\n\n[agents]\ndriver = \"not this one\"\nmine = \"mine {task}\"\n",
        )
        .unwrap();
        store.set_driver("mine").unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(
            text.starts_with("# my settings\nmax_agents = 2\ndriver = \"mine\"\n"),
            "{text}"
        );
        assert!(
            text.contains("[agents]\ndriver = \"not this one\""),
            "a key inside a table is left alone"
        );
        let config = store.config().unwrap();
        assert_eq!(config.driver, "mine");
        assert_eq!(config.max_agents, 2);
    }
}
