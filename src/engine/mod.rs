//! The reactive engine: dependency graph, dirty propagation, topological
//! recompute and cycle rejection. Holds an in-memory mirror of the cells;
//! every write goes through the store.
//!
//! `write` changes cells, `graph` recomputes them, `runs` spawns and queues
//! `$`/`!`/`@` runs, `decisions` handles questions, `agents` builds agent
//! commands, `structure` moves rows and columns, `watch` re-runs on file
//! changes, `sync` absorbs other processes' writes, `templates` loads and
//! saves boards.

use crate::interp::Form;
use crate::model::{
    a1_coord, a1_name, cell_coord, on_sheet, split_sheet, A1Ref, Actor, AnswerKind, AnswerType,
    Cell, CellKind, CellState, Cite, Decision, Format, Mark, Provenance, Suggestion,
};
use crate::store::Store;
use crate::{agent, danger, exec, expr, interp, stream};
use anyhow::{bail, Result};
use notify::Watcher;
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

mod agents;
mod decisions;
mod graph;
mod rows;
mod runs;
mod sheets;
mod structure;
mod sync;
mod templates;
#[cfg(test)]
mod tests;
mod watch;
mod write;

use graph::{glob_root, walk_matches};
use runs::{source_error, split_annotations, Annotations};
#[cfg(test)]
use structure::remap_refs;
use structure::shift_refs;
use templates::parse_csv;
use write::{answer_type_spec, parse_answer_type_spec};

pub struct Engine {
    store: Store,
    config: crate::store::Config,
    cells: HashMap<String, Cell>,
    /// Reverse edges: the cells whose deps name each cell. Kept in step with
    /// `cells` by `put`/`remove`.
    dependents: HashMap<String, BTreeSet<String>>,
    /// Each cell's name in lowercase, to its name: `Total` and `total` would
    /// share one file where file names ignore case (macOS, Windows).
    folded: HashMap<String, String>,
    /// The board's sheets, for resolving references (`qualify`); refreshed
    /// when a sheet is added, renamed, moved or deleted, here or elsewhere.
    sheets: crate::store::Sheets,
    launch_dir: PathBuf,
    /// Per-cell run generation. A finished run from an older generation was
    /// superseded and its result is discarded.
    run_gen: HashMap<String, u64>,
    in_flight: usize,
    runs_tx: mpsc::UnboundedSender<RunDone>,
    /// Progress events from streaming agent runs, tagged with cell and run generation.
    stream_tx: mpsc::UnboundedSender<(String, u64, stream::Event)>,
    stream_rx: mpsc::UnboundedReceiver<(String, u64, stream::Event)>,
    /// Per cell, the agent run waiting on the answer to the cell's open question.
    pending_asks: HashMap<String, tokio::sync::oneshot::Sender<stream::Answer>>,
    runs_rx: mpsc::UnboundedReceiver<RunDone>,
    /// The causal trace shown in the activity pane.
    activity: VecDeque<Activity>,
    /// Won the per-board election (engine.pid). Only the active engine runs `$`
    /// cells reactively; passive engines mirror its results and run commands
    /// only on explicit trigger, so two processes never both run every command.
    active: bool,
    last_election: std::time::Instant,
    /// Shut down: the window has let this board go (it's closing, switching,
    /// or moving the folder), so this engine never runs or takes over again.
    shut: bool,
    /// `config.toml` as last read, and when it was last looked at: an edit
    /// (by hand, by the Driver, by the app) applies without reopening.
    pub(super) config_stamp: Option<(std::time::SystemTime, u64)>,
    config_checked: std::time::Instant,
    /// Claude Code's state file, where it records what each folder's last
    /// session used; the Driver's runs are counted from it.
    pub(super) claude_state: Option<PathBuf>,
    claude_state_stamp: Option<(std::time::SystemTime, u64)>,
    driver_checked: std::time::Instant,
    /// Something the window shows besides cells changed (the Driver's tally).
    news: bool,
    /// One notify watcher over the paths named by `# watch:` globs; None when no
    /// cell watches. Active engine only.
    repo_watch: Option<RepoWatch>,
    /// A `$` cell changed; `pump_watched` re-derives the watch specs next tick.
    watch_dirty: bool,
    /// Whether watched `$` cells have been checked against files changed while
    /// no engine was watching. Done once, on the first compute.
    watches_caught_up: bool,
    /// The `# every:` cells and their periods, derived alongside the watches.
    everies: Vec<(String, u64)>,
    /// When each `# every:` cell is next due, on the pump clock.
    every_due: HashMap<String, std::time::Instant>,
    /// Agent runs waiting for a slot under `max_agents`, in trigger order.
    agent_queue: VecDeque<String>,
    agents_running: usize,
    /// The `@` cell another agent is answering for it (`answer_cell`), for
    /// the one write it takes.
    answering: Option<String>,
    /// Unix ms until which the agent queue holds after a rate limit. A person
    /// running an agent cell lifts it early.
    agents_paused_until: Option<u64>,
    /// Cells a person ran one at a time; they start even over budget.
    budget_exempt: HashSet<String>,
    /// Inside `trigger_many`: a person's runs there are bulk, not one-off.
    in_bulk: bool,
    /// Whether the log already says the budget holds the queue.
    budget_noted: bool,
    /// Document text by board-relative path, with the mtime it was read at, for
    /// `quote_found`.
    source_text: HashMap<String, (std::time::SystemTime, String)>,
    /// Agent nesting depth: 0 in the window, CLI and driver; 1 in the MCP server
    /// of an agent a cell started. `@` runs from here get `depth + 1`.
    depth: u32,
    /// Hand agent runs to the board's active engine when one is alive. Set for
    /// `recalc mcp`, whose client kills it when done, taking held runs with it.
    hand_off_runs: bool,
    /// The depth an adopted run keeps: that of the agent that asked for it.
    adopted_depth: HashMap<String, u32>,
    /// When runs owned by other processes were last checked for a dead owner.
    last_orphan_sweep: std::time::Instant,
    /// Stops each run in flight; firing the sender kills its process group.
    stoppers: HashMap<String, tokio::sync::oneshot::Sender<()>>,
    /// The process groups of those runs, killed when the engine goes.
    groups: exec::Groups,
    /// Cells touched since a frontend last asked. Removed cells appear here and
    /// are absent from `cells`.
    touched: HashSet<String>,
}

struct RepoWatch {
    /// The sorted (cell, glob) pairs this watcher was built from.
    specs: Vec<(String, String)>,
    matchers: Vec<Matcher>,
    /// Canonicalized launch dir; events arrive canonicalized.
    root: PathBuf,
    rx: mpsc::UnboundedReceiver<PathBuf>,
    /// None when building the OS watcher failed; the specs stay recorded so the
    /// failure is logged once rather than retried every pump.
    _watcher: Option<notify::RecommendedWatcher>,
}

struct Matcher {
    cell: String,
    glob: globset::GlobMatcher,
    /// The glob explicitly targets `.git/`; only such a glob lets git internals
    /// re-run a cell.
    git: bool,
}

/// How a pending decision is answered.
pub enum Choice {
    Index(usize),
    Text(String),
}

/// Fill direction (⌘D / ⌘R).
#[derive(Clone, Copy)]
pub enum Fill {
    Down,
    Right,
}

/// Which way a structural edit moves the sheet.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Axis {
    Rows,
    Cols,
}

/// What a row or column delete removed, for `undelete`.
#[derive(Clone, Debug, Default)]
pub struct Removed {
    /// The deleted cells, whole, under their old names.
    pub cells: Vec<Cell>,
    /// Surviving cells whose references the delete rewrote.
    pub rewritten: Vec<Rewritten>,
}

/// One source a delete rewrote, before and after.
#[derive(Clone, Debug)]
pub struct Rewritten {
    pub before: String,
    pub before_source: String,
    pub after: String,
    pub after_source: String,
}

/// What `trigger_many` did with a selection.
#[derive(Debug, Default, PartialEq)]
pub struct RunSelection {
    pub ran: Vec<String>,
    /// Cells skipped because their answer is current (`all` re-runs them).
    pub up_to_date: Vec<String>,
    /// Quarantined or destructive-looking cells, left for a person to confirm.
    pub need_confirm: Vec<String>,
}

/// What a command cell would run now, with its re-run annotations.
pub struct Console {
    pub command: String,
    pub cwd: PathBuf,
    /// `# watch:` glob, verbatim.
    pub watch: Option<String>,
    /// `# every:` period in milliseconds.
    pub every: Option<u64>,
}

struct RunDone {
    name: String,
    gen: u64,
    /// An `@` run: frees an agent slot when it lands, superseded or not.
    agent: bool,
    result: Result<exec::Output, String>,
}

impl Engine {
    pub fn open(dir: &Path) -> Result<Engine> {
        let store = Store::open(dir)?;
        let (runs_tx, runs_rx) = mpsc::unbounded_channel();
        let (stream_tx, stream_rx) = mpsc::unbounded_channel();
        let active = store.try_claim_engine();
        let config = store.config()?;
        let config_stamp = store.config_stamp();
        // Every agent launch names this file, so it must exist once the board is open.
        store.ensure_mcp_config()?;
        let mut engine = Engine {
            store,
            config,
            cells: HashMap::new(),
            dependents: HashMap::new(),
            folded: HashMap::new(),
            sheets: crate::store::Sheets::default(),
            launch_dir: dir.to_path_buf(),
            run_gen: HashMap::new(),
            in_flight: 0,
            runs_tx,
            runs_rx,
            stream_tx,
            stream_rx,
            pending_asks: HashMap::new(),
            activity: VecDeque::new(),
            active,
            last_election: std::time::Instant::now(),
            shut: false,
            config_stamp,
            config_checked: std::time::Instant::now(),
            claude_state: crate::session::claude_state_path(),
            claude_state_stamp: None,
            driver_checked: std::time::Instant::now(),
            news: false,
            repo_watch: None,
            watch_dirty: true,
            watches_caught_up: false,
            everies: Vec::new(),
            every_due: HashMap::new(),
            agent_queue: VecDeque::new(),
            agents_running: 0,
            answering: None,
            agents_paused_until: None,
            budget_exempt: HashSet::new(),
            in_bulk: false,
            budget_noted: false,
            last_orphan_sweep: std::time::Instant::now(),
            depth: 0,
            hand_off_runs: false,
            adopted_depth: HashMap::new(),
            source_text: HashMap::new(),
            stoppers: HashMap::new(),
            groups: exec::Groups::default(),
            touched: HashSet::new(),
        };
        engine.sheets = engine.store.sheets();
        for cell in engine.store.list()? {
            engine.put(cell);
        }
        engine.reset_orphaned_runs()?;
        Ok(engine)
    }

    /// The cell as it stands, for a write that changes one property. A passive
    /// engine reads it from disk: writing back from its trailing mirror would
    /// revert the value and state.
    fn current(&self, name: &str) -> Option<Cell> {
        let mirror = self.cells.get(name).cloned();
        if self.active {
            return mirror;
        }
        match self.store.get(name).ok().flatten() {
            Some(mut disk) => {
                if let Some(m) = mirror.filter(|_| self.store.owns_run(name)) {
                    disk.state = m.state;
                }
                Some(disk)
            }
            None => mirror,
        }
    }

    /// Install a cell in the mirror, keeping the reverse-dependency index and
    /// `watch_dirty` in step. Every cell replacement goes through here.
    fn put(&mut self, cell: Cell) {
        let name = cell.name.clone();
        if let Some(old) = self.cells.get(&name) {
            self.watch_dirty |= old.kind == CellKind::Command;
            for dep in &old.deps {
                if let Some(set) = self.dependents.get_mut(dep) {
                    set.remove(&name);
                }
            }
        }
        self.watch_dirty |= cell.kind == CellKind::Command;
        for dep in &cell.deps {
            self.dependents
                .entry(dep.clone())
                .or_default()
                .insert(name.clone());
        }
        self.touched.insert(name.clone());
        self.folded.insert(name.to_ascii_lowercase(), name.clone());
        self.cells.insert(name, cell);
    }

    /// An existing cell whose name differs from `name` only in capitals.
    pub(super) fn case_twin(&self, name: &str) -> Option<&str> {
        if self.cells.contains_key(name) {
            return None;
        }
        self.folded
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    /// Refuse a new cell that would share a file with an existing one.
    pub(super) fn check_case(&self, name: &str) -> Result<()> {
        match self.case_twin(name) {
            Some(twin) => bail!(
                "{name} and {twin} differ only in capital letters, so they'd share one file on this computer; use {twin}, or another name"
            ),
            None => Ok(()),
        }
    }

    fn remove(&mut self, name: &str) {
        if let Some(old) = self.cells.remove(name) {
            let lower = name.to_ascii_lowercase();
            if self.folded.get(&lower).is_some_and(|n| n == name) {
                self.folded.remove(&lower);
            }
            self.watch_dirty |= old.kind == CellKind::Command;
            for dep in &old.deps {
                if let Some(set) = self.dependents.get_mut(dep) {
                    set.remove(name);
                }
            }
            self.touched.insert(name.to_string());
        }
    }

    /// Cells changed since the last call, for frontends that push deltas.
    pub fn take_changed(&mut self) -> HashSet<String> {
        std::mem::take(&mut self.touched)
    }

    /// Whether something the window shows besides cells changed since asked.
    pub fn take_news(&mut self) -> bool {
        std::mem::take(&mut self.news)
    }

    pub fn cell(&self, name: &str) -> Option<&Cell> {
        self.cells.get(name)
    }

    pub fn all_cells(&self) -> impl Iterator<Item = &Cell> {
        self.cells.values()
    }

    pub fn dir(&self) -> &Path {
        &self.launch_dir
    }

    pub fn watch(&self) -> Result<crate::store::Watch> {
        self.store.watch()
    }

    pub fn config(&self) -> &crate::store::Config {
        &self.config
    }

    /// Whether this engine won the per-board election and runs `$` cells
    /// reactively.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Override the per-command timeout for this engine's runs.
    pub fn set_timeout(&mut self, ms: u64) {
        self.config.timeout_ms = ms;
    }

    pub fn activity(&self) -> impl DoubleEndedIterator<Item = &Activity> {
        self.activity.iter()
    }

    pub fn history(&self, name: &str, limit: usize) -> Result<Vec<(u64, String)>> {
        self.store.history(name, limit)
    }

    pub fn layout(&self) -> Result<String> {
        self.store.layout()
    }

    pub fn set_layout(&self, json: &str) -> Result<()> {
        self.store.set_layout(json)
    }

    fn log(&mut self, entry: String) {
        self.push_activity(None, entry);
    }

    /// Log what someone did: `by`'s name, then `what`.
    fn log_as(&mut self, by: &Actor, what: String) {
        let by = String::from(by.clone());
        let text = format!("{by} {what}");
        self.push_activity(Some(by), text);
    }

    fn push_activity(&mut self, by: Option<String>, text: String) {
        if self.activity.len() >= 200 {
            self.activity.pop_front();
        }
        self.activity.push_back(Activity {
            at: now_ms(),
            by,
            text,
        });
    }

    /// Log an activity line from outside the engine.
    pub fn note_activity(&mut self, entry: &str) {
        self.log(entry.to_string());
    }

    /// Absorb pending file changes and finished runs without blocking. Returns
    /// whether anything moved; frontends call this every tick.
    pub fn pump(&mut self, watch: &mut crate::store::Watch) -> Result<bool> {
        // A board let go of is left alone: taking it over again would run its
        // cells against a folder that may have moved.
        if self.shut {
            return Ok(false);
        }
        self.reload_config_if_changed();
        self.collect_driver_usage();
        // Take over if the active engine exited, so the board keeps computing.
        if !self.active && self.last_election.elapsed() >= std::time::Duration::from_secs(1) {
            self.last_election = std::time::Instant::now();
            if self.store.try_claim_engine() {
                self.active = true;
                self.watches_caught_up = false;
                self.log("took over as active engine".into());
                let stale: Vec<String> = self
                    .cells
                    .values()
                    .filter(|c| c.kind == CellKind::Command && c.state == CellState::Stale)
                    .map(|c| c.name.clone())
                    .collect();
                self.recompute(stale)?;
            }
        }
        let mut names: Vec<String> = Vec::new();
        while let Ok(name) = watch.rx.try_recv() {
            if !names.contains(&name) {
                names.push(name);
            }
        }
        let mut moved = !names.is_empty();
        // "" is the watch's word for "the sheets themselves changed".
        if names.iter().any(String::is_empty) {
            self.sheets_changed_elsewhere()?;
            names.retain(|n| !n.is_empty());
        }
        for name in names {
            self.reconcile(&name)?;
        }
        while let Ok((name, gen, ev)) = self.stream_rx.try_recv() {
            self.stream_event(name, gen, ev)?;
            moved = true;
        }
        moved |= self.collect_answers()?;
        while let Ok(done) = self.runs_rx.try_recv() {
            self.in_flight -= 1;
            self.apply_run(done)?;
            moved = true;
        }
        moved |= self.lift_expired_hold()?;
        // A run owned by another process can lose its owner while the board is
        // open, leaving its cell running forever.
        if self.last_orphan_sweep.elapsed() >= std::time::Duration::from_secs(5) {
            self.last_orphan_sweep = std::time::Instant::now();
            let before = self.touched.len();
            self.reset_orphaned_runs()?;
            moved |= self.touched.len() != before;
        }
        if self.active {
            moved |= self.pump_watched()?;
        }
        Ok(moved)
    }

    /// Pick up an edited `config.toml`. A file that doesn't parse keeps the
    /// settings in force, and the activity log says why.
    fn reload_config_if_changed(&mut self) {
        if self.config_checked.elapsed() < std::time::Duration::from_millis(500) {
            return;
        }
        self.config_checked = std::time::Instant::now();
        let stamp = self.store.config_stamp();
        if stamp == self.config_stamp {
            return;
        }
        self.config_stamp = stamp;
        match self.store.config() {
            Ok(config) => {
                self.config = config;
                self.log("settings reloaded from .recalc/config.toml".into());
            }
            Err(e) => self.log(format!(
                "config.toml didn't load, so the previous settings stay: {e:#}"
            )),
        }
    }

    /// Kill this engine's runs, return their cells to Stale and give up the
    /// board, so no untracked run lands later. Called on drop and by frontends
    /// that exit without dropping.
    pub fn shutdown(&mut self) {
        self.shut = true;
        self.groups.kill_all();
        let mine: Vec<String> = self
            .stoppers
            .drain()
            .map(|(name, _)| name)
            .chain(self.agent_queue.drain(..))
            .collect();
        let passive = !self.active;
        for name in mine {
            if let Some(cell) = self.cells.get_mut(&name) {
                // A run a passive engine only queued stays queued for the active engine's
                // orphan sweep to adopt.
                if passive && cell.state == CellState::Queued {
                    continue;
                }
                if matches!(cell.state, CellState::Computing | CellState::Queued) {
                    cell.state = CellState::Stale;
                    let _ = self.store.set_state(cell);
                }
            }
            self.store.clear_running(&name);
        }
        if std::mem::take(&mut self.active) {
            self.store.release_engine();
        }
    }

    /// Wait for in-flight runs, and what they dirty, to settle. For the CLI;
    /// long-lived frontends pump instead.
    pub async fn drain(&mut self) -> Result<()> {
        while self.in_flight > 0 {
            tokio::select! {
                done = self.runs_rx.recv() => {
                    let done = done.expect("engine holds a sender");
                    self.in_flight -= 1;
                    self.apply_run(done)?;
                }
                Some((name, gen, ev)) = self.stream_rx.recv() => self.stream_event(name, gen, ev)?,
                // An answer another process left (`recalc resolve`, the window).
                _ = tokio::time::sleep(std::time::Duration::from_millis(200)), if !self.pending_asks.is_empty() => {
                    self.collect_answers()?;
                }
            }
        }
        Ok(())
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// How deep agents may start agents; past this a loop of agents would only
/// spend money.
const MAX_AGENT_DEPTH: u32 = 3;

/// One line of the activity log: when, who (absent for recalc's own notes,
/// like a run finishing), and the line, which starts with who.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Activity {
    /// Unix milliseconds.
    pub at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    pub text: String,
}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn def_body(source: &str) -> &str {
    source.trim_start()[1..].trim()
}

/// Extract a def body's deps, each through `qualify` (the cell it names,
/// seen from the cell that reads it); a parse failure becomes the cell's error.
fn parse_deps(
    kind: CellKind,
    body: &str,
    qualify: &dyn Fn(&str) -> String,
) -> (Vec<String>, Option<String>) {
    let (deps, err) = match kind {
        CellKind::Expr => match expr::parse(body) {
            Ok(ast) => (expr::deps(&ast), None),
            Err(e) => (Vec::new(), Some(e)),
        },
        CellKind::AgentCall => match agent::deps(body) {
            Ok(deps) => (deps, None),
            Err(e) => (Vec::new(), Some(e)),
        },
        _ => {
            let (cmd, _) = split_annotations(body);
            (interp::deps(cmd), source_error(kind, body))
        }
    };
    let mut out: Vec<String> = Vec::with_capacity(deps.len());
    for dep in deps {
        let dep = qualify(&dep);
        if !out.contains(&dep) {
            out.push(dep);
        }
    }
    (out, err)
}
