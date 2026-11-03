//! Headless runner: spawns an agent on the `task` cell with recalc's MCP
//! server and serves the board until it exits.

use crate::engine::Engine;
use crate::interp::shell_quote;
use crate::model::Actor;
use anyhow::{bail, Context, Result};
use std::path::Path;

const DEFAULT_AGENT: &str = "claude -p {task} --mcp-config {mcp}";

pub async fn run(dir: &Path, task: Option<String>, agent: Option<String>) -> Result<()> {
    let mut engine = Engine::open(dir)?;
    if let Some(task) = &task {
        engine.set("task", task, Actor::Cli)?;
    }
    let task_value = engine
        .cell("task")
        .map(|c| c.value.clone())
        .unwrap_or_default();
    if task_value.is_empty() {
        bail!("no task: pass --task or set the `task` cell first");
    }

    let mcp_config = crate::store::Store::open(dir)?.ensure_mcp_config()?;

    // {mcp} first, so a task containing the text "{mcp}" isn't substituted.
    let paths = engine.config().paths.clone();
    let template = match agent {
        Some(custom) => custom,
        None => crate::agent::relocate(DEFAULT_AGENT, "claude", &paths),
    };
    let cmd = template
        .replace("{mcp}", &shell_quote(&mcp_config.to_string_lossy()))
        .replace("{task}", &shell_quote(&task_value));

    engine.set("agent_status", "running", Actor::Cli)?;
    let mut child = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        // `recalc` from the agent's shell writes as an agent (cli::me).
        .env("RECALC_AGENT", "agent")
        .current_dir(dir)
        .spawn()
        .with_context(|| format!("spawning agent: {cmd}"))?;

    let mut watch = engine.watch()?;
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(50));
    let status = loop {
        tokio::select! {
            status = child.wait() => break status?,
            _ = tick.tick() => { engine.pump(&mut watch)?; }
        }
    };
    let exit = status
        .code()
        .map_or("killed".to_string(), |c| format!("exited {c}"));
    engine.set("agent_status", &exit, Actor::Cli)?;
    engine.drain().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    #[tokio::test]
    async fn drives_an_agent_against_the_board() {
        let dir = tempfile::tempdir().unwrap();
        run(
            dir.path(),
            Some("fix the tests".into()),
            Some("mkdir -p .recalc/cells && echo working > .recalc/cells/agent_note".into()),
        )
        .await
        .unwrap();
        let store = Store::open(dir.path()).unwrap();
        assert_eq!(store.get("task").unwrap().unwrap().value, "fix the tests");
        assert_eq!(
            store.get("agent_status").unwrap().unwrap().value,
            "exited 0"
        );
        assert_eq!(store.get("agent_note").unwrap().unwrap().value, "working");
        let cfg = std::fs::read_to_string(dir.path().join(".recalc/mcp.json")).unwrap();
        assert!(cfg.contains("\"args\":[\"mcp\"]"));
    }

    #[tokio::test]
    async fn no_task_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(run(dir.path(), None, Some("true".into())).await.is_err());
    }
}
