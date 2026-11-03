//! The `recalc` subcommands, for scripts, CI and agents without MCP. The
//! desktop crate owns the binary and opens the app when none is given.

use crate::engine::Engine;
use crate::model::{Actor, CellKind};
use crate::store::Store;
use anyhow::{bail, Result};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "recalc",
    version,
    about = "A reactive grid you and your coding agent share"
)]
pub struct Args {
    /// With no subcommand, open the board in the current directory.
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(Subcommand)]
pub enum Cmd {
    /// Print a cell's value; given several cells or a range (Rules!A1:B6), each labelled
    Get {
        #[arg(required = true)]
        names: Vec<String>,
    },
    /// Set a cell: content starting with = $ ! installs a source, else a literal value
    Set { name: String, content: String },
    /// Run a ! action cell (or force-refresh a $ cell)
    Trigger { name: String },
    /// Blocking watch: print the cell's value once it changes / satisfies --until
    Watch {
        name: String,
        /// Predicate on the value, e.g. "== true", "!= ''", "> 0.9"
        #[arg(long)]
        until: Option<String>,
        #[arg(long, value_name = "MS")]
        timeout: Option<u64>,
    },
    /// Answer a pending decision: an option number (1-based) or freetext
    Resolve { name: String, choice: String },
    /// MCP server over stdio (for a coding agent to spawn)
    Mcp {
        /// Agent id stamped on this client's writes (defaults to the MCP
        /// client's name from the initialize handshake)
        #[arg(long)]
        id: Option<String>,
        /// Serve the headless agent computing this cell: its writes carry
        /// the cell's name, and it may not write or run `$`/`!` cells
        #[arg(long, conflicts_with = "id")]
        cell: Option<String>,
        /// How many agents deep this client is (recalc sets it; see `--cell`)
        #[arg(long, default_value_t = 0)]
        depth: u32,
    },
    /// Headless runner: drive an agent entirely through the grid
    Run {
        #[arg(long)]
        task: Option<String>,
        /// Agent command; {task} and {mcp} are substituted (shell-quoted)
        #[arg(long)]
        agent: Option<String>,
    },
    /// Apply a board and verify it: exit 0 only if --cell reads true.
    /// The committed-contract form for CI: `recalc check verify.recalctemplate`
    Check {
        /// Board template to apply first; omit to check the board as it is
        file: Option<std::path::PathBuf>,
        /// The cell that must read exactly "true"
        #[arg(long, default_value = "status")]
        cell: String,
        /// Per-command timeout for this run, in ms
        #[arg(long, value_name = "MS")]
        timeout: Option<u64>,
    },
    /// Load a board template (`name = content` lines; see examples/)
    Open { file: std::path::PathBuf },
    /// Import a CSV file as literal cells, top-left field at --at
    Import {
        /// The CSV file, or `-` for stdin (a script's output: `… | recalc import - --at C2`)
        file: std::path::PathBuf,
        #[arg(long, default_value = "A1")]
        at: String,
    },
    /// Write a copy of this board, and documents its cells name from elsewhere, to one zip to send
    Export { file: std::path::PathBuf },
    /// Save this board as a template others (and other agents) can load
    Save {
        file: std::path::PathBuf,
        /// One-line note on what the board is for
        #[arg(long)]
        note: Option<String>,
    },
    /// Dump all cells
    List {
        #[arg(long)]
        json: bool,
    },
    /// Hooks for coding agents. `stop` is Claude Code's Stop hook: it holds a
    /// session's "done" to its row on the board and blocks while the row fails
    #[command(subcommand)]
    Hook(HookCmd),
    /// Continue an agent cell's conversation in this terminal: join its
    /// live session (tmux), or resume it by the id its last run left.
    /// `_driver` names the board's driver.
    Attach {
        name: String,
        /// Print the command instead of running it
        #[arg(long)]
        print: bool,
    },
    /// Unpack a demo (board + sample data) into a new directory and open it;
    /// with no name, list the demos
    Demo {
        name: Option<String>,
        /// Where to unpack (default: ./recalc-NAME)
        #[arg(long)]
        into: Option<std::path::PathBuf>,
        /// Unpack and apply, but don't open the window (for scripts)
        #[arg(long)]
        no_open: bool,
    },
}

/// Parse the process arguments, exiting with usage on a mistake.
pub fn parse() -> Args {
    match Args::try_parse() {
        Ok(args) => args,
        // Exit 2 from a Stop hook blocks the agent with the message as the
        // reason, so a hook whose arguments don't parse would trap it.
        Err(e) if e.use_stderr() && std::env::args().nth(1).as_deref() == Some("hook") => {
            eprintln!("{e}");
            std::process::exit(0);
        }
        Err(e) => e.exit(),
    }
}

/// Run one subcommand against the board in the current directory. Returns
/// a directory when the command's last step is to open a window on it.
#[derive(clap::Subcommand, Debug)]
pub enum HookCmd {
    /// Read Claude Code's Stop hook input on stdin; print a block decision
    /// while the session's row doesn't pass. Find the row with --session
    /// (a column of session ids) or --task (a column naming the task, found
    /// in the session's folder)
    Stop {
        /// The board (default: the nearest one at or above the session's folder)
        #[arg(long)]
        board: Option<std::path::PathBuf>,
        /// Column of session ids; the hook writes the session's id there
        #[arg(long)]
        session: Option<String>,
        /// Column naming each row's task, matched against the session's folder
        #[arg(long)]
        task: Option<String>,
        /// Column the session's last message is written to
        #[arg(long)]
        claim: Option<String>,
        /// Column whose value decides whether the session may stop
        #[arg(long)]
        gate: String,
        /// Values of the gate that let the session stop
        #[arg(
            long,
            value_delimiter = ',',
            default_value = "holds,pass,passes,true,yes,ok"
        )]
        pass: Vec<String>,
        /// Blocks before the session may stop anyway (the gate is then flagged)
        #[arg(long, default_value_t = 3)]
        max_blocks: u32,
        /// Seconds to wait for the row's checks to settle
        #[arg(long, default_value_t = 50)]
        wait: u64,
    },
}

pub async fn run(cmd: Cmd) -> Result<Option<std::path::PathBuf>> {
    let dir = std::env::current_dir()?;
    // A Ctrl-C or kill ends the runs this command started; `recalc mcp`
    // handles its own signals, as Claude Code sends them.
    if !matches!(cmd, Cmd::Mcp { .. }) {
        tokio::spawn(crate::exec::exit_on_signal());
    }
    // A hook runs wherever the session is; it finds its board itself, and it
    // never fails the session.
    if let Cmd::Hook(HookCmd::Stop {
        board,
        session,
        task,
        claim,
        gate,
        pass,
        max_blocks,
        wait,
    }) = cmd
    {
        let mut stdin = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut stdin)?;
        let args = crate::hook::StopArgs {
            board,
            session,
            task,
            claim,
            gate,
            pass,
            max_blocks,
            wait_secs: wait,
        };
        match crate::hook::stop(args, &stdin).await {
            Ok(Some(decision)) => println!("{decision}"),
            Ok(None) => {}
            Err(e) => eprintln!("recalc hook stop: {e:#}"),
        }
        return Ok(None);
    }
    if let Cmd::Demo {
        name,
        into,
        no_open,
    } = cmd
    {
        let opened = demo(&dir, name, into).await?;
        return Ok(if no_open { None } else { opened });
    }
    let store = Store::open(&dir)?;
    match cmd {
        Cmd::Get { names } => {
            // One cell: its value alone, and one range its rows alone (TSV),
            // for scripts. More than one: each labelled.
            if let [name] = names.as_slice() {
                if !name.contains(':') {
                    let name = store.canonical(name);
                    let Some(cell) = store.get(&name)? else {
                        // Say which: a sheet that isn't there, or a cell nothing was written to.
                        if let (Some(sheet), _) = crate::model::split_sheet(&name) {
                            let sheets = store.sheets();
                            if sheets.find(sheet).is_none() {
                                let all: Vec<String> =
                                    std::iter::once(sheets.first).chain(sheets.others).collect();
                                bail!(
                                    "no sheet named {sheet}; this board's sheets are {}",
                                    all.join(", ")
                                );
                            }
                        }
                        bail!("{name} is empty: nothing has been written to it");
                    };
                    println!("{}", cell.value);
                    return Ok(None);
                }
            }
            let value = |n: &str| -> Result<String> {
                Ok(store.get(n)?.map(|c| c.value).unwrap_or_default())
            };
            for arg in &names {
                match arg.split_once(':') {
                    Some((from, to)) => {
                        let from = store.canonical(from);
                        let Some((sheet, c1, r1)) = crate::model::cell_coord(&from) else {
                            bail!("{arg} isn't a range like Rules!A1:B6");
                        };
                        let Some((_, c2, r2)) = crate::model::cell_coord(&store.canonical(to))
                        else {
                            bail!("{arg} isn't a range like Rules!A1:B6");
                        };
                        if names.len() > 1 {
                            println!("== {arg} ==");
                        }
                        for r in r1.min(r2)..=r1.max(r2) {
                            let row: Vec<String> = (c1.min(c2)..=c1.max(c2))
                                .map(|c| {
                                    value(&crate::model::on_sheet(
                                        sheet,
                                        &crate::model::a1_name(c, r),
                                    ))
                                })
                                .collect::<Result<_>>()?;
                            // A row with nothing in it says nothing.
                            if row.iter().any(|v| !v.is_empty()) {
                                println!("{}", row.join("\t").replace('\n', " "));
                            }
                        }
                    }
                    None => {
                        let name = store.canonical(arg);
                        let v = value(&name)?;
                        // A value of several lines starts on its own.
                        if v.contains('\n') {
                            println!("{arg}:\n{v}");
                        } else {
                            println!("{arg}: {v}");
                        }
                    }
                }
            }
        }
        Cmd::Set { name, content } => {
            let name = store.canonical(&name);
            let mut engine = Engine::open(&dir)?;
            engine.set(&name, &content, me())?;
            engine.drain().await?;
        }
        Cmd::Trigger { name } => {
            let name = store.canonical(&name);
            let mut engine = Engine::open(&dir)?;
            engine.trigger(&name, me())?;
            engine.drain().await?;
        }
        Cmd::Watch {
            name,
            until,
            timeout,
        } => {
            let name = store.canonical(&name);
            let deadline =
                timeout.map(|ms| std::time::Instant::now() + std::time::Duration::from_millis(ms));
            let initial = store.get(&name)?.map(|c| c.value);
            loop {
                let value = store.get(&name)?.map(|c| c.value);
                let met = match &until {
                    Some(predicate) => {
                        let lookup = |dep: &str| {
                            store
                                .get(dep)
                                .ok()
                                .flatten()
                                .map(|c| c.value)
                                .unwrap_or_default()
                        };
                        crate::expr::predicate(&name, predicate, &lookup)
                            .map_err(|e| anyhow::anyhow!(e))?
                    }
                    None => value != initial,
                };
                if met {
                    println!("{}", value.unwrap_or_default());
                    return Ok(None);
                }
                if deadline.is_some_and(|d| std::time::Instant::now() >= d) {
                    bail!("timeout waiting on {name}");
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
        Cmd::Resolve { name, choice } => {
            let name = store.canonical(&name);
            let mut engine = Engine::open(&dir)?;
            let choice = match choice.parse::<usize>() {
                Ok(n) if n >= 1 => crate::engine::Choice::Index(n - 1),
                _ => crate::engine::Choice::Text(choice),
            };
            engine.resolve_decision(&name, choice, me())?;
            engine.drain().await?;
        }
        Cmd::Mcp { id, cell, depth } => {
            let cell_agent = cell.is_some();
            // A cell's live session exports RECALC_AGENT: its writes carry the
            // cell's name, as its shell's do, so an answer in its own cell is one.
            let id = id.or_else(|| match me() {
                Actor::Agent(id) => Some(id),
                _ => None,
            });
            crate::mcp::serve_stdio(&dir, cell.or(id), cell_agent, depth).await?;
            // The server's engine has shut down. Exit now: after a signal, the
            // runtime would otherwise wait on the stdin read for as long as the
            // client keeps the pipe open.
            std::process::exit(0);
        }
        Cmd::Attach { name, print } => {
            let name = if name == crate::session::DRIVER {
                name
            } else {
                store.canonical(&name)
            };
            let tname = crate::session::name(&dir, &name);
            let cmd = if crate::session::available() && crate::session::alive(&tname) {
                crate::session::attach_command(&tname)
            } else if name == crate::session::DRIVER {
                bail!("the driver is not running; start it from the window's Driver tab");
            } else {
                let engine = Engine::open(&dir)?;
                // A board from another computer names conversations this one never had.
                let program = engine.program_of(&name).unwrap_or_default();
                if let Some(id) = engine.cell(&name).and_then(|c| c.session_id.clone()) {
                    if !crate::session::conversation_exists(program, &id) {
                        bail!("{name}: its last conversation isn't on this computer; open its session in the window to start a new one, then recalc attach {name}");
                    }
                }
                engine
                    .resume_info(&name)
                    .map(|(cmd, _)| cmd)
                    .map_err(|e| anyhow::anyhow!("{name}: {e}"))?
            };
            if print {
                println!("{cmd}");
            } else {
                let status = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(&cmd)
                    .current_dir(&dir)
                    .status()?;
                if !status.success() {
                    bail!("{cmd}: exited {}", status.code().unwrap_or(-1));
                }
            }
        }
        Cmd::Run { task, agent } => crate::runner::run(&dir, task, agent).await?,
        Cmd::Check {
            file,
            cell,
            timeout,
        } => check(&dir, file.as_deref(), &cell, timeout).await?,
        Cmd::Demo { .. } | Cmd::Hook(_) => unreachable!("handled above"),
        Cmd::Open { file } => {
            let text = std::fs::read_to_string(&file)?;
            let mut engine = Engine::open(&dir)?;
            engine
                .apply_template(&text, me())
                .map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?;
            engine.drain().await?;
        }
        Cmd::Import { file, at } => {
            // `-` reads the CSV from stdin, so a script can pipe its output in.
            let text = if file.as_os_str() == "-" {
                let mut s = String::new();
                std::io::Read::read_to_string(&mut std::io::stdin(), &mut s)?;
                s
            } else {
                std::fs::read_to_string(&file)
                    .map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?
            };
            let mut engine = Engine::open(&dir)?;
            let changed = engine.import_csv(&text, &at, me())?;
            engine.drain().await?;
            // On stderr: a script that pipes its rows in keeps its own output.
            eprintln!("imported {} cells at {at}", changed.len());
        }
        Cmd::Save { file, note } => {
            let engine = Engine::open(&dir)?;
            let text = engine.save_template(note.as_deref());
            if let Some(parent) = file.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&file, &text)?;
            // Confirm, so a caller can tell the file was written.
            let cells = engine.all_cells().count();
            let noun = if cells == 1 { "cell" } else { "cells" };
            println!("saved {cells} {noun} to {}", file.display());
        }
        Cmd::Export { file } => {
            // Opening the store writes `.recalc/.gitignore` on a board from before it.
            crate::store::Store::open(&dir)?;
            let file = crate::bundle::as_zip(&file);
            let done = crate::bundle::export(&dir, &file)?;
            println!("exported a copy: {}", crate::bundle::summary(&done, &file));
        }
        Cmd::List { json } => {
            let cells = store.list()?;
            if json {
                let out = serde_json::to_value(&cells)?;
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                for cell in &cells {
                    println!("{}", row(cell));
                }
            }
        }
    }
    Ok(None)
}

/// `recalc demo [name]`: list the demos, or unpack one and apply its board.
async fn demo(
    cwd: &std::path::Path,
    name: Option<String>,
    into: Option<std::path::PathBuf>,
) -> Result<Option<std::path::PathBuf>> {
    let Some(name) = name else {
        for d in crate::demos::DEMOS {
            println!("{:<10} {}", d.name, d.blurb);
        }
        println!("\nrecalc demo <name> [--into DIR]");
        return Ok(None);
    };
    let Some(demo) = crate::demos::find(&name) else {
        let names: Vec<&str> = crate::demos::DEMOS.iter().map(|d| d.name).collect();
        bail!("no demo named {name:?}; there are: {}", names.join(", "));
    };
    let target = into.unwrap_or_else(|| cwd.join(format!("recalc-{name}")));
    let board = crate::demos::unpack(demo, &target)?;
    let mut engine = Engine::open(&target)?;
    engine.apply_template(board, me())?;
    engine.drain().await?;
    println!("{} unpacked into {}", demo.name, target.display());
    Ok(Some(target))
}

/// `recalc check`. On failure, every cell is printed to stderr for the CI log.
async fn check(
    dir: &std::path::Path,
    file: Option<&std::path::Path>,
    cell: &str,
    timeout: Option<u64>,
) -> Result<()> {
    let mut engine = Engine::open(dir)?;
    if !engine.is_active() {
        bail!("another recalc process holds this board; checks would not run");
    }
    if let Some(ms) = timeout {
        engine.set_timeout(ms);
    }
    if let Some(file) = file {
        let text = std::fs::read_to_string(file)
            .map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?;
        engine
            .apply_template(&text, me())
            .map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?;
    }
    engine.drain().await?;
    // A cell that errored keeps its last value, so passing also requires state Ok.
    let (verdict, state) = engine.cell(cell).map_or_else(
        || {
            (
                String::new(),
                crate::model::CellState::Error("no such cell".into()),
            )
        },
        |c| (c.value.clone(), c.state.clone()),
    );
    if verdict == "true" && state == crate::model::CellState::Ok {
        println!("{cell} = true");
        return Ok(());
    }
    let mut cells: Vec<_> = engine.all_cells().collect();
    cells.sort_by(|a, b| a.name.cmp(&b.name));
    for cell in cells {
        eprintln!("{}", row(cell));
    }
    if state != crate::model::CellState::Ok {
        bail!(
            "{cell} = {verdict:?} but its state is {} (want \"true\", ok)",
            state.word()
        );
    }
    bail!("{cell} = {verdict:?} (want \"true\")");
}

/// Who a command acts as. Shells recalc starts for agents set `RECALC_AGENT`,
/// so their writes get the same rules as that agent's MCP writes.
fn me() -> Actor {
    match std::env::var("RECALC_AGENT") {
        Ok(id) if !id.is_empty() => Actor::Agent(id),
        _ => Actor::Cli,
    }
}

fn row(cell: &crate::model::Cell) -> String {
    format!(
        "{:<20} {:<8} {:<10} {}",
        cell.name,
        kind_word(cell.kind),
        cell.state.word(),
        first_line(&cell.value),
    )
}

fn kind_word(kind: CellKind) -> &'static str {
    match kind {
        CellKind::Literal => "literal",
        CellKind::Expr => "expr",
        CellKind::Command => "command",
        CellKind::Action => "action",
        CellKind::AgentCall => "agent",
    }
}

fn first_line(value: &str) -> String {
    match value.split_once('\n') {
        Some((first, _)) => format!("{first} …"),
        None => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn check_passes_and_fails_by_the_target_cell() {
        let dir = tempfile::tempdir().unwrap();
        let board = dir.path().join("board.recalctemplate");
        std::fs::write(&board, "failed = $ echo 0\nstatus = = {failed} == 0\n").unwrap();
        check(dir.path(), Some(&board), "status", None)
            .await
            .unwrap();

        // The same contract fails when the check disagrees.
        std::fs::write(&board, "failed = $ echo 2\nstatus = = {failed} == 0\n").unwrap();
        let err = check(dir.path(), Some(&board), "status", None)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("\"false\""), "{err}");

        // An errored gate keeps its last value but must not pass.
        std::fs::write(&board, "n = 1\nstatus = = 1 / {n} == 1\n").unwrap();
        check(dir.path(), Some(&board), "status", None)
            .await
            .unwrap();
        std::fs::write(&board, "n = 0\n").unwrap();
        let err = check(dir.path(), Some(&board), "status", None)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("division by zero"), "{err}");
        assert!(err.contains("\"true\""), "{err}");

        // A missing target cell is a plain failure, not a panic.
        let err = check(dir.path(), None, "ghost", None)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("\"\""), "{err}");
    }
}
