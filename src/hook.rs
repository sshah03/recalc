//! `recalc hook stop`: Claude Code's `Stop` hook, so a session's "done" is held
//! to its row on a board. The hook finds the session's row (the agent cell a
//! session recalc started names in `RECALC_AGENT`, a session-id column, or a
//! task name in the session's folder), writes the session id and
//! its last message there, waits for the row's checks to settle, and blocks the
//! stop with the failing check's reason while the row's verdict isn't passing.
//! After `max_blocks` blocks it lets the session stop and flags the verdict.
//! It never fails the session: any error is printed to stderr and the stop goes on.

use crate::engine::Engine;
use crate::model::{
    a1_coord, a1_name, cell_coord, on_sheet, Actor, Cell, CellKind, CellState, Mark, MarkKind,
};
use crate::store::Store;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Where on the board a session's row lives, from the command line.
#[derive(Debug, Clone)]
pub struct StopArgs {
    pub board: Option<PathBuf>,
    /// Column holding each row's session id; the hook fills it in.
    pub session: Option<String>,
    /// Column whose value names the row's task, found in the session's folder.
    pub task: Option<String>,
    /// Column the session's last message is written to.
    pub claim: Option<String>,
    /// Column whose value decides: the stop goes on only when it passes.
    pub gate: String,
    pub pass: Vec<String>,
    pub max_blocks: u32,
    pub wait_secs: u64,
}

/// What Claude Code sends a `Stop` hook on stdin (the fields used here).
#[derive(serde::Deserialize, Default)]
struct StopInput {
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    cwd: Option<PathBuf>,
    #[serde(default)]
    transcript_path: Option<PathBuf>,
    #[serde(default)]
    last_assistant_message: Option<String>,
}

/// Run the hook: read stdin, print a block decision or nothing.
pub async fn stop(args: StopArgs, stdin: &str) -> Result<Option<String>> {
    // A cell's headless run answers the cell; only live sessions are held.
    if std::env::var_os("RECALC_HEADLESS").is_some() {
        return Ok(None);
    }
    let input: StopInput = serde_json::from_str(stdin).context("reading the hook's input")?;
    let cwd = input.cwd.clone().unwrap_or(std::env::current_dir()?);
    let board = match &args.board {
        Some(b) => b.clone(),
        None => find_board(&cwd).context("no recalc board in or above the session's folder")?,
    };
    let col = |c: &str| -> Result<u32> {
        a1_coord(&format!("{c}1"))
            .map(|(c, _)| c)
            .with_context(|| format!("{c:?} isn't a column letter"))
    };
    let gate_col = col(&args.gate)?;
    let mut engine = Engine::open(&board)?;
    let Some((sheet, row)) = find_row(&engine, &args, &input.session_id, &cwd, &col)? else {
        return Ok(None); // not a session this board tracks
    };
    // The row's cells, on the sheet the session's cell is on.
    let at = |c: u32| on_sheet(sheet.as_deref(), &a1_name(c, row));
    let started = crate::engine::now_ms();
    let who = Actor::Agent(format!("session {}", short(&input.session_id)));
    let mut writes = Vec::new();
    if let Some(c) = &args.session {
        writes.push((at(col(c)?), input.session_id.clone()));
    }
    if let Some(c) = &args.claim {
        let said = input
            .last_assistant_message
            .clone()
            .or_else(|| input.transcript_path.as_deref().and_then(last_message))
            .unwrap_or_default();
        writes.push((at(col(c)?), clip(&said, 1000)));
    }
    if !writes.is_empty() {
        engine.set_many(writes, who.clone())?;
    }
    engine.rerun_changed_checks()?;
    engine.drain().await?;
    drop(engine);

    // Settle: nothing in the row computing, the gate recomputed since the claim.
    let store = Store::open(&board)?;
    let gate = at(gate_col);
    let deadline = Instant::now() + Duration::from_secs(args.wait_secs);
    let row_cells = |store: &Store| -> Result<Vec<Cell>> {
        let mut out = Vec::new();
        for c in 1..=last_col(store, sheet.as_deref())? {
            if let Some(cell) = store.get(&at(c))? {
                out.push(cell);
            }
        }
        Ok(out)
    };
    let mut quiet = 0;
    loop {
        let cells = row_cells(&store)?;
        let busy = cells
            .iter()
            .any(|c| matches!(c.state, CellState::Computing | CellState::Queued));
        let gate_fresh = args.claim.is_none()
            || cells
                .iter()
                .any(|c| c.name == gate && c.provenance.at >= started);
        quiet = if !busy && gate_fresh { quiet + 1 } else { 0 };
        if quiet >= 2 {
            break;
        }
        if Instant::now() >= deadline {
            eprintln!(
                "recalc: {gate}'s row didn't settle in {}s; letting the session stop",
                args.wait_secs
            );
            return Ok(None);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    let cells = row_cells(&store)?;
    let verdict = store.get(&gate)?.map(|c| c.value).unwrap_or_default();
    if args
        .pass
        .iter()
        .any(|p| p.eq_ignore_ascii_case(verdict.trim()))
    {
        forget_blocks(&board, &input.session_id);
        return Ok(None);
    }
    let blocks = count_block(&board, &input.session_id)?;
    if blocks > args.max_blocks {
        let mut engine = Engine::open(&board)?;
        engine.set_mark(
            &gate,
            Some(Mark {
                kind: MarkKind::Flagged,
                note: Some(format!(
                    "the session stopped with {gate} reading {:?} after {} tries",
                    verdict.trim(),
                    args.max_blocks
                )),
                by: who,
                at: crate::engine::now_ms(),
            }),
        )?;
        forget_blocks(&board, &input.session_id);
        return Ok(None);
    }
    let mut reason = format!(
        "recalc: {gate} reads {:?}, not {}.",
        verdict.trim(),
        args.pass.join(" or ")
    );
    for c in cells
        .iter()
        .filter(|c| c.kind == CellKind::Command && c.exit_code.is_some_and(|x| x != 0))
    {
        let out = if c.stderr.trim().is_empty() {
            &c.value
        } else {
            &c.stderr
        };
        let last = out
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim();
        reason.push_str(&format!(
            " {} exits {}: {}",
            c.name,
            c.exit_code.unwrap_or(-1),
            clip(last, 300)
        ));
    }
    if !reason.ends_with('.') {
        reason.push('.');
    }
    reason.push_str(" Fix it and finish again; the board re-checks when files change.");
    Ok(Some(
        serde_json::json!({ "decision": "block", "reason": reason }).to_string(),
    ))
}

fn find_board(from: &Path) -> Option<PathBuf> {
    from.ancestors()
        .find(|d| d.join(".recalc").is_dir())
        .map(Path::to_path_buf)
}

fn find_row(
    engine: &Engine,
    args: &StopArgs,
    session: &str,
    cwd: &Path,
    col: &dyn Fn(&str) -> Result<u32>,
) -> Result<Option<(Option<String>, u32)>> {
    // Every row from 2 down on every sheet, as (sheet, row).
    let rows = || {
        let mut rows: Vec<(Option<String>, u32)> = engine
            .all_cells()
            .filter_map(|c| cell_coord(&c.name).map(|(s, _, r)| (s.map(str::to_string), r)))
            .filter(|(_, r)| *r >= 2)
            .collect();
        rows.sort_unstable();
        rows.dedup();
        rows
    };
    let value = |sheet: &Option<String>, c: u32, r: u32| {
        engine
            .cell(&on_sheet(sheet.as_deref(), &a1_name(c, r)))
            .map(|c| c.value.trim().to_string())
    };
    // A session recalc started names its agent cell (`RECALC_AGENT`); that
    // cell's row, on its sheet, is its row.
    if let Ok(agent) = std::env::var("RECALC_AGENT") {
        if let Some((sheet, _, r)) = cell_coord(&agent) {
            if r >= 2 && engine.cell(&on_sheet(sheet, &a1_name(1, r))).is_some() {
                return Ok(Some((sheet.map(str::to_string), r)));
            }
        }
    }
    if let Some(s) = &args.session {
        let c = col(s)?;
        if let Some(found) = rows()
            .into_iter()
            .find(|(sh, r)| value(sh, c, *r).as_deref() == Some(session) && !session.is_empty())
        {
            return Ok(Some(found));
        }
    }
    if let Some(t) = &args.task {
        let c = col(t)?;
        let parts: Vec<String> = cwd
            .components()
            .map(|p| p.as_os_str().to_string_lossy().into_owned())
            .collect();
        return Ok(rows().into_iter().find(|(sh, r)| {
            value(sh, c, *r).is_some_and(|v| !v.is_empty() && parts.contains(&v))
        }));
    }
    // Not a session this board started, and no other way to find its row.
    Ok(None)
}

fn last_col(store: &Store, sheet: Option<&str>) -> Result<u32> {
    Ok(store
        .list()?
        .iter()
        .filter_map(|c| cell_coord(&c.name))
        .filter(|(s, _, _)| *s == sheet)
        .map(|(_, c, _)| c)
        .max()
        .unwrap_or(1))
}

/// The last assistant text in a Claude Code transcript (JSONL).
fn last_message(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines().rev().find_map(|line| {
        let v: serde_json::Value = serde_json::from_str(line).ok()?;
        if v["type"] != "assistant" {
            return None;
        }
        let said: Vec<&str> = v["message"]["content"]
            .as_array()?
            .iter()
            .filter(|p| p["type"] == "text")
            .filter_map(|p| p["text"].as_str())
            .collect();
        (!said.is_empty()).then(|| said.join("\n"))
    })
}

fn clip(s: &str, max: usize) -> String {
    let s = s.trim();
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

fn short(session: &str) -> &str {
    session.get(..8).unwrap_or(session)
}

fn blocks_file(board: &Path, session: &str) -> PathBuf {
    let safe: String = session
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    board.join(".recalc/hooks").join(format!("stop-{safe}"))
}

/// Count this block for the session; returns the count including it.
fn count_block(board: &Path, session: &str) -> Result<u32> {
    let file = blocks_file(board, session);
    let n = std::fs::read_to_string(&file)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or(0)
        + 1;
    std::fs::create_dir_all(file.parent().expect("has a parent"))?;
    std::fs::write(&file, n.to_string())?;
    Ok(n)
}

fn forget_blocks(board: &Path, session: &str) {
    let _ = std::fs::remove_file(blocks_file(board, session));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(board: &Path) -> StopArgs {
        StopArgs {
            board: Some(board.to_path_buf()),
            session: Some("B".into()),
            task: Some("A".into()),
            claim: Some("C".into()),
            gate: "E".into(),
            pass: vec!["holds".into()],
            max_blocks: 3,
            wait_secs: 20,
        }
    }

    #[tokio::test]
    async fn a_session_may_stop_only_when_its_row_holds() {
        let dir = tempfile::tempdir().unwrap();
        let board = dir.path();
        std::fs::create_dir_all(board.join("wt/rounding")).unwrap();
        std::fs::write(board.join("wt/rounding/answer.txt"), "41").unwrap();
        {
            let mut e = Engine::open(board).unwrap();
            e.set_many(
                vec![
                    ("A2".into(), "rounding".into()),
                    ("D2".into(), "$ test \"$(cat wt/rounding/answer.txt)\" = 42 || { echo 'expected 42' >&2; exit 1; } # watch: wt/rounding/**".into()),
                    ("E2".into(), "= if({C2} == '', 'pending', if({D2.code} == 0, 'holds', 'STALE'))".into()),
                ],
                Actor::Human,
            )
            .unwrap();
            e.drain().await.unwrap();
        }
        let input = |id: &str| {
            serde_json::json!({
                "session_id": id,
                "cwd": board.join("wt/rounding"),
                "last_assistant_message": "Done: the answer is fixed.",
            })
            .to_string()
        };
        let blocked = stop(args(board), &input("s-1"))
            .await
            .unwrap()
            .expect("blocks while red");
        assert!(blocked.contains("\"decision\":\"block\""), "{blocked}");
        assert!(
            blocked.contains("STALE") && blocked.contains("expected 42"),
            "{blocked}"
        );
        let store = Store::open(board).unwrap();
        assert_eq!(store.get("B2").unwrap().unwrap().value, "s-1");
        assert_eq!(
            store.get("C2").unwrap().unwrap().value,
            "Done: the answer is fixed."
        );

        // The agent fixes it; the next stop goes through.
        std::fs::write(board.join("wt/rounding/answer.txt"), "42").unwrap();
        assert_eq!(stop(args(board), &input("s-1")).await.unwrap(), None);

        // A session in no tracked folder is left alone.
        let other = serde_json::json!({ "session_id": "s-2", "cwd": board }).to_string();
        assert_eq!(stop(args(board), &other).await.unwrap(), None);

        // Red again, and the session keeps finishing: after three blocks it may stop, flagged.
        std::fs::write(board.join("wt/rounding/answer.txt"), "40").unwrap();
        for _ in 0..3 {
            assert!(stop(args(board), &input("s-1")).await.unwrap().is_some());
        }
        assert_eq!(stop(args(board), &input("s-1")).await.unwrap(), None);
        let gate = store.get("E2").unwrap().unwrap();
        assert_eq!(gate.mark.map(|m| m.kind), Some(MarkKind::Flagged));
    }
}
