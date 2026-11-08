//! The `@` agent-call cell: parse `@claude('prompt', {refs}, {A1:B3})`,
//! build the agent's task text, and build the command that runs it with the
//! board's MCP server. Execution is in `exec`.

use crate::interp::{self, shell_quote, Form, Piece};
use crate::model::{range_cells, ref_name, Usage};
use serde_json::Value;
use std::collections::HashMap;

/// A parsed `@agent('prompt', ...)` call.
#[derive(Debug)]
pub struct Call {
    pub agent: String,
    /// The prompt verbatim, `{refs}` unresolved.
    pub prompt: String,
    /// `{ref}`/`{A1:B3}` arguments expanded to cell names, appended to the
    /// task as `name = value` lines.
    pub context: Vec<String>,
    /// `model='haiku'`: the model to run, an alias from `[models]` or an id
    /// the agent takes. None runs the agent's default.
    pub model: Option<String>,
}

/// Headless templates; `[agents]` in config.toml adds to and overrides them.
/// Placeholders are substituted shell-quoted: `{task}` the prompt, `{mcp}`
/// the MCP config path, `{recalc}` this binary, `{mcpargs}` the `recalc mcp`
/// arguments as a JSON array (naming the cell for a headless run), `{piext}`
/// the pi extension that connects pi to the config (`Store::write_mcp_config`).
const BUILTIN: &[(&str, &str)] = &[
    // Claude: the prompt goes on stdin; other tools are asked of the person
    // over stream-json (`stream::run`). Reads are pre-approved only inside
    // the board's directory, so a document can't get files outside read
    // unasked. `--strict-mcp-config` keeps the person's own MCP connectors
    // out, so a headless run can change nothing but cells.
    // Codex: `exec` refuses every MCP call unless recalc's tools are
    // pre-approved.
    // Both print machine-readable output for `parse_output`, so cost and
    // tokens land on the cell.
    (
        "claude",
        "claude -p {model} {dirs} --input-format stream-json --output-format stream-json --verbose --permission-prompt-tool stdio --mcp-config {mcp} --strict-mcp-config --allowedTools mcp__recalc 'Read(./**)' 'Grep(./**)' 'Glob(./**)'",
    ),
    (
        "codex",
        "codex exec {model} --skip-git-repo-check --json -c mcp_servers.recalc.command={recalc} -c mcp_servers.recalc.args={mcpargs} -c 'mcp_servers.recalc.default_tools_approval_mode=\"approve\"' -",
    ),
    // pi: the prompt goes on stdin and events come out as JSON lines. pi has no
    // flag for an MCP config, so `{piext}`, an extension recalc writes beside
    // the config, connects it. A headless run can read and search files and
    // use the board's tools: no shell, no edits.
    (
        "pi",
        "pi --mode json {model} -e {piext} --recalc-mcp {mcp} --tools read,grep,find,ls,'mcp__recalc__*'",
    ),
];

/// Interactive templates for live sessions in a PTY. `{session}` is the
/// conversation id recalc mints for the cell; Codex has no flag to take one.
/// Session workers get recalc's MCP server only; the driver and resumed
/// conversations keep the person's own connectors. Codex passes MCP servers
/// only the variables it's told to, so it forwards `RECALC_AGENT` (the cell
/// the session works for) to recalc's.
const SESSION_BUILTIN: &[(&str, &str)] = &[
    (
        "claude",
        "claude {task} {model} {dirs} --session-id {session} --mcp-config {mcp} --strict-mcp-config",
    ),
    (
        "codex",
        "codex {model} -c mcp_servers.recalc.command={recalc} -c 'mcp_servers.recalc.args=[\"mcp\"]' -c 'mcp_servers.recalc.env_vars=[\"RECALC_AGENT\"]' -c 'mcp_servers.recalc.default_tools_approval_mode=\"approve\"' {task}",
    ),
    (
        "pi",
        "pi {model} --session-id {session} -e {piext} --recalc-mcp {mcp} {task}",
    ),
];

/// Templates that resume a cell's conversation in a terminal. A custom
/// template that runs `claude` or `codex exec` resumes with these too.
const RESUME_BUILTIN: &[(&str, &str)] = &[
    ("claude", "claude --resume {session} {model} {dirs} --mcp-config {mcp}"),
    (
        "codex",
        "codex resume {model} {session} -c mcp_servers.recalc.command={recalc} -c 'mcp_servers.recalc.args=[\"mcp\"]' -c 'mcp_servers.recalc.env_vars=[\"RECALC_AGENT\"]' -c 'mcp_servers.recalc.default_tools_approval_mode=\"approve\"'",
    ),
    ("pi", "pi --session {session} {model} -e {piext} --recalc-mcp {mcp}"),
];

/// Templates for the driver: an interactive agent with no cell, connected as
/// `driver`. `{prompt}` is the path of the role text; Codex gets the text as
/// `{instructions}`.
const DRIVER_BUILTIN: &[(&str, &str)] = &[
    (
        "claude",
        "claude --session-id {session} --mcp-config {mcp} --append-system-prompt-file {prompt}",
    ),
    (
        "codex",
        "codex -c mcp_servers.recalc.command={recalc} -c 'mcp_servers.recalc.args=[\"mcp\", \"--id\", \"driver\"]' -c 'mcp_servers.recalc.default_tools_approval_mode=\"approve\"' -c {instructions}",
    ),
    (
        "pi",
        "pi --session-id {session} -e {piext} --recalc-mcp {mcp} --append-system-prompt {prompt}",
    ),
];

/// Templates that continue the driver's conversation after its terminal is
/// gone (a restart). A custom Claude driver resumes as its own template.
const DRIVER_RESUME_BUILTIN: &[(&str, &str)] = &[
    (
        "claude",
        "claude --resume {session} --mcp-config {mcp} --append-system-prompt-file {prompt}",
    ),
    (
        "codex",
        "codex resume {session} -c mcp_servers.recalc.command={recalc} -c 'mcp_servers.recalc.args=[\"mcp\", \"--id\", \"driver\"]' -c 'mcp_servers.recalc.default_tools_approval_mode=\"approve\"' -c {instructions}",
    ),
    (
        "pi",
        "pi --session {session} -e {piext} --recalc-mcp {mcp} --append-system-prompt {prompt}",
    ),
];

/// The driver's role text. Board habits come from the MCP server's
/// instructions; this holds only what differs for the driver.
pub const DRIVER_PROMPT: &str = include_str!("prompts/driver.md");

/// The arguments after the prompt: `, {ref}, {A1:B3}` as cell names, and
/// `model='…'`.
fn context_args(mut rest: &str) -> Result<Vec<String>, String> {
    let mut context = Vec::new();
    while !rest.is_empty() {
        rest = rest
            .strip_prefix(',')
            .ok_or("expected , between arguments")?
            .trim_start();
        if rest.starts_with("model") {
            return Err("model= goes last, after the prompt and any {refs}".into());
        }
        let arg = rest
            .strip_prefix('{')
            .and_then(|r| r.split_once('}'))
            .ok_or("context arguments are {refs} or {A1:B3} ranges")?;
        match arg.0.split_once(':') {
            Some((from, to)) => context.extend(
                range_cells(from.trim(), to.trim())
                    .ok_or_else(|| format!("bad range {{{}}}", arg.0))?,
            ),
            None => match ref_name(arg.0) {
                Some(name) => context.push(name),
                None => return Err(format!("bad reference {{{}}}", arg.0)),
            },
        }
        rest = arg.1.trim_start();
    }
    Ok(context)
}

/// A trailing `, model='…'`, taken off the call's arguments before the
/// prompt's end is found, so its quotes can't be mistaken for the prompt's.
fn trailing_model(inner: &str) -> Result<(&str, Option<String>), String> {
    let re = regex::Regex::new(r#",\s*model\s*=\s*(?:'([^']*)'|"([^"]*)")\s*$"#).expect("valid");
    let Some(m) = re.captures(inner) else {
        return Ok((inner, None));
    };
    let id = m.get(1).or(m.get(2)).map_or("", |g| g.as_str());
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._:/-[]".contains(c))
    {
        return Err(format!("{id:?} isn't a model name"));
    }
    Ok((
        &inner[..m.get(0).expect("whole match").start()],
        Some(id.to_string()),
    ))
}

/// Parse the text after the `@` prefix: `name('prompt'[, {ref}...])`.
pub fn parse(body: &str) -> Result<Call, String> {
    let body = body.trim();
    let open = body
        .find('(')
        .ok_or("expected @agent('prompt', {refs}...)")?;
    let agent = body[..open].trim();
    let inner = body[open + 1..]
        .strip_suffix(')')
        .ok_or("expected closing )")?
        .trim();
    let (inner, model) = trailing_model(inner)?;
    let inner = inner.trim_end();

    let quote = match inner.chars().next() {
        Some(q @ ('\'' | '"')) => q,
        _ => return Err("the prompt must be a quoted string".into()),
    };
    // The prompt ends at the last quote followed only by `, {ref}`
    // arguments, so apostrophes in the prose don't end it.
    let closes: Vec<usize> = inner[1..].match_indices(quote).map(|(i, _)| i).collect();
    if closes.is_empty() {
        return Err("unterminated prompt string".into());
    }
    let end = closes
        .iter()
        .rev()
        .copied()
        .find(|&e| context_args(inner[2 + e..].trim_start()).is_ok())
        .ok_or_else(|| {
            let e = closes[0];
            let after = inner[2 + e..].trim_start();
            match context_args(after) {
                Err(why) => format!(
                    "{why}; the prompt ended at the {quote} before {:?}",
                    after.chars().take(24).collect::<String>()
                ),
                Ok(_) => "unterminated prompt string".to_string(),
            }
        })?;
    let prompt = inner[1..1 + end].to_string();
    let context = context_args(inner[2 + end..].trim_start())?;
    // A model= placed before the {refs} would have read as part of the prompt.
    if regex::Regex::new(r#"['"]\s*,\s*model\s*=\s*['"]"#)
        .expect("valid")
        .is_match(&prompt)
    {
        return Err("model= goes last, after the prompt and any {refs}".into());
    }

    // Repo refs must stay inside the board, as in command cells.
    if let Some(e) = interp::repo_ref_error(&prompt) {
        return Err(e);
    }

    Ok(Call {
        agent: agent.to_string(),
        prompt,
        context,
        model,
    })
}

/// Every cell the call reads: refs inside the prompt, then context members.
pub fn deps(body: &str) -> Result<Vec<String>, String> {
    let call = parse(body)?;
    let mut out = interp::deps(&call.prompt);
    for name in &call.context {
        if !out.contains(name) {
            out.push(name.clone());
        }
    }
    Ok(out)
}

/// The agent's task: the prompt with `{refs}` interpolated unquoted, plus a
/// `name = value` block for the context arguments.
pub fn task_text(call: &Call, lookup: &dyn Fn(&str, Form) -> String) -> String {
    let mut out = String::new();
    for piece in interp::parse(&call.prompt) {
        match piece {
            Piece::Text(t) => out.push_str(&t),
            // A range reads as rows, cells joined by ` | `.
            Piece::Ref {
                name,
                form: Form::Range,
            } => {
                let rows: Vec<String> = interp::range_rows(&name)
                    .iter()
                    .map(|row| {
                        row.iter()
                            .map(|m| lookup(m, Form::Quoted))
                            .collect::<Vec<_>>()
                            .join(" | ")
                    })
                    .collect();
                out.push_str(&rows.join("\n"));
            }
            Piece::Ref { name, form } => out.push_str(&lookup(&name, form)),
        }
    }
    if !call.context.is_empty() {
        out.push_str("\n\nGrid context:\n");
        for name in &call.context {
            out.push_str(&format!("{name} = {}\n", lookup(name, Form::Quoted)));
        }
    }
    out
}

/// The text appended to every agent cell's task: its own name (needed to
/// cite or check itself), how its answer reaches the cell, and how this agent
/// reads files. A headless run's printed answer becomes the value; a live
/// session's printing is only on its screen, so it sets the cell instead.
/// Codex has no Read tool, so it is told to use its shell. Table-specific
/// conventions belong in the board's `instructions` cell, not here.
pub fn footer(name: &str, agent: &str, live: bool, tools: bool) -> String {
    let read = match agent {
        "claude" => "Read files with the Read and Grep tools; a shell command needs a person's permission and waits for it. ",
        "codex" => "Read files with your shell (cat, sed, rg); reading needs no one's permission. ",
        "pi" => "Read files with the read, grep, find and ls tools; reading needs no one's permission. ",
        _ => "",
    };
    let answer = if live {
        "this is a live session, so what you print isn't saved: when you have the answer, write it with the recalc tool set_cell, name=\"{name}\" (that sets its value and keeps this prompt)."
    } else {
        "what you print becomes its value, so print only the answer."
    };
    // An agent run without recalc's tools would only search for them.
    if !tools {
        return format!("\n\n(You are computing cell {name} of the recalc board in this directory; {answer} {read})")
            .replace(" )", ")");
    }
    format!(
        "\n\n{}",
        FOOTER
            .trim_end()
            .replace("{answer}", answer)
            .replace("{name}", name)
            .replace("{read}", read)
    )
}

/// Whether a run of `agent` can reach the board: the builtins always do, a
/// custom `[agents]` template only when it passes recalc's MCP server.
pub fn has_board_tools(agent: &str, setup: Setup<'_>) -> bool {
    match setup.agents.get(agent) {
        Some(t) => ["{mcp}", "{mcpargs}", "{recalc}"]
            .iter()
            .any(|p| t.contains(p)),
        None => true,
    }
}

/// The message for an agent whose program isn't installed: what to install,
/// then the `[paths]` setting.
pub fn missing_program(agent: &str) -> String {
    let install = match agent {
        "claude" => "Claude Code isn't installed, or recalc can't find it. Install it (https://claude.com/claude-code, or `npm install -g @anthropic-ai/claude-code`) and run `claude` once to sign in",
        "codex" => "Codex isn't installed, or recalc can't find it. Install it (`npm install -g @openai/codex`) and run `codex` once to sign in",
        "pi" => "pi isn't installed, or recalc can't find it. Install it (https://pi.dev, or `npm install -g @earendil-works/pi-coding-agent`), then run `pi` once and /login to sign in",
        _ => return format!("{agent}'s command wasn't found; check its template under [agents] in .recalc/config.toml"),
    };
    format!("{install}. If it's installed somewhere unusual, set `{agent} = \"/path/to/{agent}\"` under [paths] in .recalc/config.toml.")
}

/// Whether a command line's program exists, as a path or on PATH.
pub fn program_exists(command: &str) -> bool {
    // `export RECALC_AGENT=…; claude …`: the program follows the export.
    let command = match command.strip_prefix("export ") {
        Some(rest) => rest.split_once("; ").map_or(rest, |(_, after)| after),
        None => command,
    };
    let Some(program) = command.split_whitespace().next() else {
        return false;
    };
    let program = program.trim_matches(|c| c == '\'' || c == '"');
    if program.contains('/') {
        return std::path::Path::new(program).is_file();
    }
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

/// The footer template; `{answer}`, `{name}` and `{read}` are filled in by `footer`.
const FOOTER: &str = include_str!("prompts/footer.md");

/// The opening of every generated review prompt: a reference to the grid's
/// shared `instructions` cell.
pub const REVIEW_INSTRUCTIONS: &str = include_str!("prompts/review_instructions.md");

/// An agent run's output: the answer, its usage, and the conversation id
/// if printed (for resuming).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Parsed {
    pub text: String,
    pub usage: Option<Usage>,
    pub session_id: Option<String>,
}

/// A failure the agent reported. `rate_limited` makes the engine hold the
/// rest of the agent queue.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentError {
    pub message: String,
    pub rate_limited: bool,
    /// When the limit lifts, in Unix seconds, if the agent said.
    pub resets_at: Option<u64>,
}

impl std::fmt::Display for AgentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// The next local time named by "resets 4:10am" or "resets 9pm", in Unix
/// seconds. Claude Code states the reset in the person's own time zone.
fn resets_today_at(text: &str) -> Option<u64> {
    let lower = text.to_ascii_lowercase();
    let rest = &lower[lower.find("resets ")? + "resets ".len()..];
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == ':'))
        .unwrap_or(rest.len());
    let word = &rest[..end];
    let (clock, pm) = match (word.strip_suffix("am"), word.strip_suffix("pm")) {
        (Some(c), _) => (c, false),
        (_, Some(c)) => (c, true),
        _ => return None,
    };
    let (h, m) = match clock.split_once(':') {
        Some((h, m)) => (h.parse::<i32>().ok()?, m.parse::<i32>().ok()?),
        None => (clock.parse::<i32>().ok()?, 0),
    };
    if !(1..=12).contains(&h) || !(0..60).contains(&m) {
        return None;
    }
    let hour = h % 12 + if pm { 12 } else { 0 };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs() as libc::time_t;
    // SAFETY: localtime_r and mktime only read and write the tm passed in.
    let at = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&now, &mut tm);
        tm.tm_hour = hour;
        tm.tm_min = m;
        tm.tm_sec = 0;
        libc::mktime(&mut tm)
    };
    if at < 0 {
        return None;
    }
    let at = if at <= now { at + 24 * 3600 } else { at };
    Some(at as u64)
}

/// Classify a failure by HTTP status and text. Readable text is kept; an
/// empty text or a bare code like `rate_limit` is replaced by advice.
fn classify(status: Option<u64>, text: &str) -> AgentError {
    let text = text.trim();
    // Older Claude Code wrote "Claude AI usage limit reached|1719876000".
    let (text, resets_at) = match text.rsplit_once('|') {
        Some((msg, secs)) if secs.len() >= 9 && secs.bytes().all(|b| b.is_ascii_digit()) => {
            (msg.trim(), secs.parse().ok())
        }
        _ => (text, None),
    };
    let resets_at = resets_at.or_else(|| resets_today_at(text));
    let lower = text.to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| lower.contains(w));
    let bare = text.is_empty() || text.bytes().all(|b| b.is_ascii_lowercase() || b == b'_');
    // Claude Code reports exhausted usage in prose, not only as a 429.
    let rate_limited = status == Some(429)
        || has(&[
            "rate limit",
            "rate_limit",
            "usage limit",
            "usage_limit",
            "limit reached",
            "session limit",
            "weekly limit",
            "hit your limit",
            "out of usage credits",
            "usage credits",
            "out of credits",
        ]);
    let plain = if rate_limited {
        "The agent's account hit its usage limit. Queued agent cells wait until it lifts; run one to try sooner."
    } else if matches!(status, Some(401) | Some(403))
        || has(&[
            "authentication",
            "not logged in",
            "invalid api key",
            "/login",
        ])
    {
        "The agent isn't signed in, or its key was refused. Run it once in a terminal to log in (for Claude Code: claude, then /login)."
    } else if status == Some(402) || has(&["billing", "credit balance"]) {
        "The agent's account has a billing problem (it may be out of credit)."
    } else if matches!(status, Some(529) | Some(503)) || has(&["overloaded"]) {
        "The model provider is overloaded. Run the cell again in a minute."
    } else if status == Some(404) || has(&["model_not_found", "model not found"]) {
        "The model this agent asked for isn't available to its account."
    } else {
        "The agent reported an error with no message; its console has the full output."
    };
    let message = if bare {
        plain.to_string()
    } else {
        text.to_string()
    };
    AgentError {
        message,
        rate_limited,
        resets_at,
    }
}

/// The model that did most of a Claude run, by cost: Claude Code also calls
/// small models for side tasks. Its canonical id, without a `[1m]` context tag.
fn main_model(usage: &Value) -> Option<String> {
    let (key, info) = usage.as_object()?.iter().max_by(|a, b| {
        let cost = |v: &Value| v["costUSD"].as_f64().unwrap_or(0.0);
        cost(a.1).total_cmp(&cost(b.1))
    })?;
    let id = info["canonicalModel"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| key.split('[').next().unwrap_or(key).to_string());
    (!id.is_empty()).then_some(id)
}

/// Parse an agent's stdout: Claude Code's result object, Codex's or pi's JSONL
/// events, or else plain text as the answer. Err is the agent's own error.
pub fn parse_output(stdout: &str) -> Result<Parsed, AgentError> {
    let text = stdout.trim();
    if let Some(obj) = text
        .strip_prefix('{')
        .and_then(|_| serde_json::from_str::<Value>(text).ok())
        .filter(|v| v.get("result").is_some() || v.get("is_error").is_some())
    {
        let result = obj["result"]
            .as_str()
            .unwrap_or_default()
            .trim()
            .to_string();
        let usage = obj.get("usage").map(|u| Usage {
            input_tokens: u["input_tokens"].as_u64().unwrap_or(0)
                + u["cache_read_input_tokens"].as_u64().unwrap_or(0)
                + u["cache_creation_input_tokens"].as_u64().unwrap_or(0),
            output_tokens: u["output_tokens"].as_u64().unwrap_or(0),
            cost_usd: obj["total_cost_usd"].as_f64(),
            turns: obj["num_turns"].as_u64().map(|n| n as u32),
            model: main_model(&obj["modelUsage"]),
        });
        if obj["is_error"].as_bool() == Some(true) {
            let mut e = classify(obj["api_error_status"].as_u64(), &result);
            // Set by `stream::run` when a rate-limit event said "rejected".
            if obj["rate_limited"].as_bool() == Some(true) {
                e.rate_limited = true;
                e.resets_at = e.resets_at.or(obj["rate_limit_resets_at"].as_u64());
            }
            return Err(e);
        }
        return Ok(Parsed {
            text: result,
            usage,
            session_id: obj["session_id"].as_str().map(str::to_string),
        });
    }
    // JSONL: every line an object with a `type`.
    let lines: Vec<Value> = text
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l.trim()).ok())
        .filter(|v| v.get("type").is_some())
        .collect();
    if !lines.is_empty() && lines.len() == text.lines().filter(|l| !l.trim().is_empty()).count() {
        let mut answer = None;
        let mut usage = None;
        let mut turns = 0u32;
        let mut error = None;
        let mut session_id = None;
        for ev in &lines {
            match ev["type"].as_str().unwrap_or_default() {
                "thread.started" => {
                    session_id = ev["thread_id"].as_str().map(str::to_string);
                }
                "item.completed" if ev["item"]["type"] == "agent_message" => {
                    answer = ev["item"]["text"].as_str().map(|s| s.trim().to_string());
                }
                "turn.completed" => {
                    turns += 1;
                    let u = &ev["usage"];
                    let prev: Usage = usage.take().unwrap_or_default();
                    usage = Some(Usage {
                        input_tokens: prev.input_tokens + u["input_tokens"].as_u64().unwrap_or(0),
                        output_tokens: prev.output_tokens
                            + u["output_tokens"].as_u64().unwrap_or(0),
                        cost_usd: None,
                        turns: Some(turns),
                        model: None,
                    });
                }
                // pi (`--mode json`): its session header, then each assistant
                // message whole as it ends, with that response's usage.
                "session" => {
                    session_id = ev["id"].as_str().map(str::to_string);
                }
                "message_end" if ev["message"]["role"] == "assistant" => {
                    let m = &ev["message"];
                    let u = &m["usage"];
                    turns += 1;
                    let prev: Usage = usage.take().unwrap_or_default();
                    let n = |k: &str| u[k].as_u64().unwrap_or(0);
                    let cost = u["cost"]["total"].as_f64();
                    usage = Some(Usage {
                        input_tokens: prev.input_tokens
                            + n("input")
                            + n("cacheRead")
                            + n("cacheWrite"),
                        output_tokens: prev.output_tokens + n("output"),
                        cost_usd: match (prev.cost_usd, cost) {
                            (None, None) => None,
                            (a, b) => Some(a.unwrap_or(0.0) + b.unwrap_or(0.0)),
                        },
                        turns: Some(turns),
                        model: m["responseModel"]
                            .as_str()
                            .or(m["model"].as_str())
                            .map(str::to_string)
                            .or(prev.model),
                    });
                    if m["stopReason"] == "error" {
                        // pi passes an HTTP failure on as `429: <the API's body>`.
                        let raw = m["errorMessage"].as_str().unwrap_or_default();
                        let (status, body) = match raw.split_once(": ") {
                            Some((code, body))
                                if code.len() == 3 && code.bytes().all(|b| b.is_ascii_digit()) =>
                            {
                                (code.parse().ok(), body)
                            }
                            _ => (None, raw),
                        };
                        let inner = serde_json::from_str::<Value>(body).ok();
                        let text = inner
                            .as_ref()
                            .and_then(|v| v["error"]["message"].as_str().or(v["message"].as_str()))
                            .unwrap_or(body);
                        error = Some(classify(status, text));
                    } else {
                        let text: Vec<&str> = m["content"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter(|b| b["type"] == "text")
                            .filter_map(|b| b["text"].as_str())
                            .collect();
                        let text = text.join("\n");
                        if !text.trim().is_empty() {
                            answer = Some(text.trim().to_string());
                        }
                    }
                }
                "turn.failed" | "error" => {
                    let raw = ev["error"]["message"]
                        .as_str()
                        .or(ev["message"].as_str())
                        .unwrap_or_default();
                    // Codex passes the API's error body through as a JSON
                    // string: `{"status":429,"error":{"message":…}}`.
                    let inner = serde_json::from_str::<Value>(raw).ok();
                    let status = inner.as_ref().and_then(|v| v["status"].as_u64());
                    let text = inner
                        .as_ref()
                        .and_then(|v| v["error"]["message"].as_str())
                        .unwrap_or(raw);
                    error = Some(classify(status, text));
                }
                _ => {}
            }
        }
        if let Some(e) = error {
            if answer.is_none() {
                return Err(e);
            }
        }
        if let Some(answer) = answer {
            return Ok(Parsed {
                text: answer,
                usage,
                session_id,
            });
        }
        // A whole run with nothing to say: its events aren't an answer.
        if session_id.is_some() {
            return Err(AgentError {
                message: "The agent finished without an answer. Resume its conversation to see what it did, or run it again.".into(),
                rate_limited: false,
                resets_at: None,
            });
        }
    }
    Ok(Parsed {
        text: text.to_string(),
        ..Parsed::default()
    })
}

/// Config for agent commands: `[agents]` templates override the builtins;
/// `[paths]` relocates a builtin's executable.
#[derive(Clone, Copy)]
pub struct Setup<'a> {
    pub agents: &'a HashMap<String, String>,
    pub paths: &'a HashMap<String, String>,
    /// `[models]` aliases for `model=`.
    pub models: &'a HashMap<String, String>,
    /// `[resume]` commands for custom templates.
    pub resume: &'a HashMap<String, String>,
    /// Folders outside the board holding the cell's input documents; `{dirs}`
    /// lets the agent read there (`--add-dir`).
    pub dirs: &'a [String],
}

/// The model id a call asks for, its `[models]` alias resolved. None runs the
/// agent's default.
pub fn resolved_model(call: &Call, setup: Setup<'_>) -> Option<String> {
    let m = call.model.as_ref()?;
    Some(setup.models.get(m).cloned().unwrap_or_else(|| m.clone()))
}

/// `{model}` in a template: `--model <id>` (Claude Code and Codex both take
/// it), or nothing for the default. A call that names a model needs a
/// template that takes one.
fn with_model(template: &str, call: &Call, setup: Setup<'_>) -> Result<String, String> {
    match resolved_model(call, setup) {
        Some(_) if !template.contains("{model}") => Err(format!(
            "@{} can't take model=: its template under [agents] has no {{model}}; add {{model}} where the model flag goes",
            call.agent
        )),
        Some(m) => Ok(template.replace("{model}", &format!("--model {}", shell_quote(&m)))),
        None => Ok(template
            .replace("{model} ", "")
            .replace(" {model}", "")
            .replace("{model}", "")),
    }
}

/// `{dirs}` in a template: `--add-dir <folder>` for each folder outside the
/// board that holds one of the cell's documents, or nothing.
fn with_dirs(template: &str, setup: Setup<'_>) -> String {
    let flags: Vec<String> = setup
        .dirs
        .iter()
        .map(|d| format!("--add-dir {}", shell_quote(d)))
        .collect();
    if flags.is_empty() {
        template.replace("{dirs} ", "").replace("{dirs}", "")
    } else {
        template.replace("{dirs}", &flags.join(" "))
    }
}

/// A command builder over a parsed call.
pub type Build<'a> = &'a dyn Fn(&Call, Setup<'_>, &str, &str) -> Result<String, String>;

/// The shell command for a headless run of the call.
pub fn command(
    call: &Call,
    setup: Setup<'_>,
    task: &str,
    mcp_config: &str,
    cell: &str,
    depth: u32,
) -> Result<String, String> {
    let template = with_model(&template(&call.agent, setup, BUILTIN)?, call, setup)?;
    let template = with_dirs(&template, setup);
    let args = serde_json::json!(["mcp", "--cell", cell, "--depth", depth.to_string()]).to_string();
    Ok(substitute(&template, mcp_config, &args, task, "", ""))
}

/// The shell command for an interactive session under conversation id
/// `session` (ignored by Codex).
pub fn session_command(
    call: &Call,
    setup: Setup<'_>,
    task: &str,
    mcp_config: &str,
    session: &str,
) -> Result<String, String> {
    let template = with_model(&template(&call.agent, setup, SESSION_BUILTIN)?, call, setup)?;
    let template = with_dirs(&template, setup);
    Ok(substitute(
        &template, mcp_config, MCP_ARGS, task, session, "",
    ))
}

/// How recalc talks to a headless run of an agent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Protocol {
    /// Claude Code's stream-json (`stream::run`): activity, and questions
    /// for the person mid-run. The prompt goes on stdin.
    ClaudeStream,
    /// Codex's `exec --json` events (`stream::run_codex`): activity only.
    /// The prompt goes on stdin (`exec -`).
    CodexEvents,
    /// pi's `--mode json` events (`stream::run_pi`): activity only. The
    /// prompt goes on stdin.
    PiEvents,
    /// Anything else: run it, read what it printed.
    Plain,
}

/// How recalc talks to an agent's run. A custom template written the way the
/// builtin is (the prompt on stdin, events out) talks as the builtin does, so
/// its run shows what it's doing and can ask for permission; any other is
/// `Plain`: its printed output is the answer.
pub fn protocol(agent: &str, setup: Setup<'_>) -> Protocol {
    match setup.agents.get(agent) {
        Some(t) => match runs_builtin(t) {
            Some((_, "claude", _))
                if t.contains("--input-format stream-json")
                    && t.contains("--output-format stream-json")
                    && !t.contains("{task}") =>
            {
                Protocol::ClaudeStream
            }
            Some((_, "codex", _))
                if t.contains("--json")
                    && t.trim_end().ends_with(" -")
                    && !t.contains("{task}") =>
            {
                Protocol::CodexEvents
            }
            Some((_, "pi", _)) if t.contains("--mode json") && !t.contains("{task}") => {
                Protocol::PiEvents
            }
            _ => Protocol::Plain,
        },
        None => match agent {
            "claude" => Protocol::ClaudeStream,
            "codex" => Protocol::CodexEvents,
            "pi" => Protocol::PiEvents,
            _ => Protocol::Plain,
        },
    }
}

/// Whether `agent`'s session template takes recalc's conversation id.
/// Codex picks its own, so recording ours would name nothing.
pub fn session_takes_id(agent: &str, setup: Setup<'_>) -> bool {
    template(agent, setup, SESSION_BUILTIN).is_ok_and(|t| t.contains("{session}"))
}

/// The shell command that resumes the call's conversation `session` in a
/// terminal, on the model the call asked for.
pub fn resume_command(
    call: &Call,
    setup: Setup<'_>,
    session: &str,
    mcp_config: &str,
) -> Result<String, String> {
    let agent = call.agent.as_str();
    let template = match setup.agents.get(agent) {
        Some(custom) => custom_resume(agent, custom, setup)?,
        None => {
            let (_, t) = RESUME_BUILTIN
                .iter()
                .find(|(n, _)| *n == agent)
                .ok_or_else(|| {
                    format!("recalc knows how to resume claude, codex and pi, not {agent:?}")
                })?;
            relocate(t, agent, setup.paths)
        }
    };
    let template = with_dirs(&with_model(&template, call, setup)?, setup);
    Ok(substitute(&template, mcp_config, MCP_ARGS, "", session, ""))
}

/// How a custom template's conversation continues: its `[resume]` entry, or,
/// for a template that runs `claude`, `codex exec` or `pi` (after any `cd dir &&`),
/// that program's resume from the same directory, with the board's tools only
/// if the template gave them.
fn custom_resume(agent: &str, template: &str, setup: Setup<'_>) -> Result<String, String> {
    if let Some(r) = setup.resume.get(agent) {
        return Ok(r.clone());
    }
    let (prefix, program, spelled) = runs_builtin(template).ok_or_else(|| {
        format!(
            "{agent} is a custom [agents] template recalc can't resume; say how under [resume] in .recalc/config.toml, e.g. {agent} = \"mytool --resume {{session}}\""
        )
    })?;
    let resume = match (program, has_board_tools(agent, setup)) {
        (_, true) => RESUME_BUILTIN
            .iter()
            .find(|(n, _)| *n == program)
            .map_or("", |(_, t)| t),
        ("claude", false) => "claude --resume {session} {model}",
        ("pi", false) => "pi --session {session} {model}",
        (_, false) => "codex resume {model} {session}",
    };
    Ok(format!("{prefix}{spelled}{}", &resume[program.len()..]))
}

/// The builtin program an agent's conversations belong to (`claude`, `codex`
/// or `pi`), for finding them on disk. None for a custom template that runs
/// something else.
pub fn program_of(agent: &str, setup: Setup<'_>) -> Option<&'static str> {
    match setup.agents.get(agent) {
        Some(t) => runs_builtin(t).map(|(_, program, _)| program),
        None => ["claude", "codex", "pi"].into_iter().find(|p| *p == agent),
    }
}

/// A custom template's `cd dir && ` prefix, the builtin it runs (`claude`,
/// `pi`, or `codex` for `codex exec`), and how the template spells that program.
fn runs_builtin(template: &str) -> Option<(&str, &'static str, &str)> {
    let mut rest = template.trim_start();
    while rest.starts_with("cd ") {
        rest = rest[rest.find("&&")? + 2..].trim_start();
    }
    let prefix = &template[..template.len() - rest.len()];
    let mut words = rest.split_whitespace();
    let spelled = words.next()?;
    match spelled.rsplit('/').next()? {
        "claude" => Some((prefix, "claude", spelled)),
        "pi" => Some((prefix, "pi", spelled)),
        "codex" if words.next() == Some("exec") => Some((prefix, "codex", spelled)),
        _ => None,
    }
}

/// The shell command for the board's driver. A custom `[agents]` template
/// gets the role text as `{task}`; `prompt_file` holds it for Claude.
pub fn driver_command(
    agent: &str,
    setup: Setup<'_>,
    mcp_config: &str,
    role: &str,
    prompt_file: &str,
    session: &str,
) -> Result<String, String> {
    let template = template(agent, setup, DRIVER_BUILTIN).map_err(|_| {
        format!(
            "driver = {agent:?} in .recalc/config.toml names no agent recalc knows; use claude, codex, pi, or a name under [agents]"
        )
    })?;
    Ok(fill_driver(
        &template,
        mcp_config,
        role,
        prompt_file,
        session,
    ))
}

/// The shell command that continues the driver's conversation `session`.
pub fn driver_resume_command(
    agent: &str,
    setup: Setup<'_>,
    mcp_config: &str,
    role: &str,
    prompt_file: &str,
    session: &str,
) -> Result<String, String> {
    let template = match setup.agents.get(agent) {
        Some(custom) => custom_driver_resume(agent, custom, setup)?,
        None => {
            let (_, t) = DRIVER_RESUME_BUILTIN
                .iter()
                .find(|(n, _)| *n == agent)
                .ok_or_else(|| {
                    format!("recalc knows how to resume claude, codex and pi, not {agent:?}")
                })?;
            relocate(t, agent, setup.paths)
        }
    };
    Ok(fill_driver(
        &template,
        mcp_config,
        role,
        prompt_file,
        session,
    ))
}

/// How a custom driver's conversation continues: its `[resume]` entry, or,
/// for a Claude template that names its conversation (`--session-id
/// {session}`), the same template resuming it, so its model, tools and board
/// connection carry over.
fn custom_driver_resume(agent: &str, template: &str, setup: Setup<'_>) -> Result<String, String> {
    if let Some(r) = setup.resume.get(agent) {
        return Ok(r.clone());
    }
    match runs_builtin(template) {
        Some((_, "claude", _)) if template.contains("--session-id {session}") => {
            Ok(template.replacen("--session-id {session}", "--resume {session}", 1))
        }
        _ => Err(format!(
            "{agent} is a custom [agents] template recalc can't resume; say how under [resume] in .recalc/config.toml, e.g. {agent} = \"mytool --resume {{session}}\""
        )),
    }
}

/// Whether the driver's template takes recalc's conversation id. Codex picks
/// its own, which recalc finds after it starts.
pub fn driver_takes_id(agent: &str, setup: Setup<'_>) -> bool {
    template(agent, setup, DRIVER_BUILTIN).is_ok_and(|t| t.contains("{session}"))
}

fn fill_driver(
    template: &str,
    mcp_config: &str,
    role: &str,
    prompt_file: &str,
    session: &str,
) -> String {
    let cmd = substitute(
        template,
        mcp_config,
        r#"["mcp","--id","driver"]"#,
        role.trim(),
        session,
        prompt_file,
    );
    // As developer instructions, not a first message, so Codex waits for
    // the person. A JSON string is valid TOML. Substituted last so the role
    // text can't contain placeholders.
    let instructions = format!(
        "developer_instructions={}",
        serde_json::to_string(role.trim()).expect("a string serializes")
    );
    cmd.replace("{instructions}", &shell_quote(&instructions))
}

/// `agent`'s template from `[agents]`, else the builtin relocated by `[paths]`.
fn template(agent: &str, setup: Setup<'_>, builtin: &[(&str, &str)]) -> Result<String, String> {
    match setup.agents.get(agent) {
        Some(custom) => Ok(custom.clone()),
        None => {
            let (_, t) = builtin.iter().find(|(n, _)| *n == agent).ok_or_else(|| {
                format!(
                    "unknown agent {agent:?}: builtins are claude, codex and pi; add `{agent} = \"...\"` under [agents] in .recalc/config.toml"
                )
            })?;
            Ok(relocate(t, agent, setup.paths))
        }
    }
}

/// `{mcpargs}` for a client that isn't a headless cell run.
const MCP_ARGS: &str = r#"["mcp"]"#;

/// Fill a template's placeholders, shell-quoted. `{task}` goes last so a
/// literal `{mcp}` in the task isn't substituted.
fn substitute(
    template: &str,
    mcp_config: &str,
    mcp_args: &str,
    task: &str,
    session: &str,
    prompt_file: &str,
) -> String {
    let recalc = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "recalc".to_string());
    // Written beside every MCP config (`Store::write_mcp_config`).
    let piext = std::path::Path::new(mcp_config)
        .with_file_name(crate::store::PI_EXTENSION)
        .to_string_lossy()
        .into_owned();
    template
        .replace("{piext}", &shell_quote(&piext))
        .replace("{mcpargs}", &shell_quote(mcp_args))
        .replace("{mcp}", &shell_quote(mcp_config))
        .replace("{recalc}", &shell_quote(&recalc))
        .replace("{session}", &shell_quote(session))
        .replace("{prompt}", &shell_quote(prompt_file))
        .replace("{task}", &shell_quote(task))
}

/// Replace a builtin template's leading program name with its `[paths]` entry.
pub fn relocate(template: &str, agent: &str, paths: &HashMap<String, String>) -> String {
    match (paths.get(agent), template.strip_prefix(agent)) {
        (Some(path), Some(rest)) => format!("{}{rest}", shell_quote(path)),
        _ => template.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_prompt_refs_and_ranges() {
        let call = parse("claude('summarize {failed}', {A1:A3}, {notes})").unwrap();
        assert_eq!(call.agent, "claude");
        assert_eq!(call.prompt, "summarize {failed}");
        assert_eq!(call.context, ["A1", "A2", "A3", "notes"]);
        // An apostrophe in the prose is part of the prompt.
        let apostrophe =
            parse("claude('round to the report's precision; it's fine', {A1})").unwrap();
        assert_eq!(
            apostrophe.prompt,
            "round to the report's precision; it's fine"
        );
        assert_eq!(apostrophe.context, ["A1"]);
        assert_eq!(
            parse("claude('the client's file')").unwrap().prompt,
            "the client's file"
        );
        // A prompt that ends early says where.
        let err = parse("claude('a' b)").unwrap_err();
        assert!(err.contains("expected , between arguments"), "{err}");
        // Trailing possessive, quote-comma-brace inside the prose, nested
        // double quotes, parentheses.
        assert_eq!(
            parse("claude('list the clients'')").unwrap().prompt,
            "list the clients'"
        );
        let inner = parse("claude('it's {B2}', and the client's {C2}', {A1})").unwrap();
        assert_eq!(inner.prompt, "it's {B2}', and the client's {C2}");
        assert_eq!(inner.context, ["A1"]);
        let dq = parse(r#"codex("say "done" when it's green", {A1:A2})"#).unwrap();
        assert_eq!(dq.prompt, r#"say "done" when it's green"#);
        assert_eq!(dq.context, ["A1", "A2"]);
        assert_eq!(
            parse("claude('check (the firm's) totals', {A1})")
                .unwrap()
                .prompt,
            "check (the firm's) totals"
        );
        // Malformed sources still fail, with a reason.
        for bad in [
            "claude('it's open)",
            "claude('a', b)",
            "claude('a', {A1}",
            "claude('a', {A1} {A2})",
            "claude(a)",
            "claude('a', {A1:})",
        ] {
            assert!(parse(bad).is_err(), "{bad} should not parse");
        }
        assert!(parse("claude('a' b)")
            .unwrap_err()
            .contains("ended at the '"));
        let anchored = parse("claude('doc {$A2}: {B$1}', {$A$1:$B$2})").unwrap();
        assert_eq!(anchored.context, ["A1", "B1", "A2", "B2"]);
        assert_eq!(deps("claude('doc {$A2}: {B$1}')").unwrap(), ["A2", "B1"]);
        assert_eq!(
            deps("claude('summarize {failed}', {A1:A2})").unwrap(),
            ["failed", "A1", "A2"]
        );
    }

    #[test]
    fn output_parses_claude_json_codex_jsonl_and_plain_text() {
        let claude = r#"{"type":"result","subtype":"success","is_error":false,"num_turns":1,"result":"pong","total_cost_usd":0.0172,"usage":{"input_tokens":10,"cache_creation_input_tokens":7316,"cache_read_input_tokens":13971,"output_tokens":48}}"#;
        let parsed = parse_output(claude).unwrap();
        assert_eq!(parsed.text, "pong");
        assert_eq!(parsed.session_id, None, "no session_id printed, none read");
        let usage = parsed.usage.unwrap();
        assert_eq!(usage.input_tokens, 10 + 7316 + 13971);
        assert_eq!(usage.output_tokens, 48);
        assert_eq!(usage.cost_usd, Some(0.0172));
        assert_eq!(usage.turns, Some(1));

        let codex = "{\"type\":\"thread.started\",\"thread_id\":\"x\"}\n{\"type\":\"turn.started\"}\n{\"type\":\"item.completed\",\"item\":{\"id\":\"item_0\",\"type\":\"agent_message\",\"text\":\"pong\"}}\n{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":14619,\"cached_input_tokens\":12160,\"output_tokens\":5}}\n";
        let parsed = parse_output(codex).unwrap();
        assert_eq!(parsed.text, "pong");
        assert_eq!(
            parsed.session_id.as_deref(),
            Some("x"),
            "codex's thread id is the session"
        );
        let usage = parsed.usage.unwrap();
        assert_eq!((usage.input_tokens, usage.output_tokens), (14619, 5));
        assert_eq!(usage.cost_usd, None);

        // Claude's session id is on the result object.
        let with_id = r#"{"type":"result","is_error":false,"result":"ok","session_id":"5f1e-abc","usage":{"input_tokens":1,"output_tokens":1}}"#;
        assert_eq!(
            parse_output(with_id).unwrap().session_id.as_deref(),
            Some("5f1e-abc")
        );

        // Plain text is the answer as printed; a stray brace is not JSON.
        assert_eq!(
            parse_output("42\n").unwrap(),
            Parsed {
                text: "42".into(),
                ..Parsed::default()
            }
        );
        assert_eq!(parse_output("{not json").unwrap().text, "{not json");
        // An agent's error report is an error, not a value.
        let err = r#"{"type":"result","is_error":true,"result":"Not logged in"}"#;
        let e = parse_output(err).unwrap_err();
        assert_eq!(
            e.message, "Not logged in",
            "a readable report is kept as is"
        );
        assert!(!e.rate_limited);
    }

    #[test]
    fn parse_errors_are_readable() {
        assert!(parse("claude").unwrap_err().contains("expected"));
        assert!(parse("claude(prompt)").unwrap_err().contains("quoted"));
        assert!(parse("claude('p' {a})").unwrap_err().contains(","));
        assert!(parse("claude('p', notaref)")
            .unwrap_err()
            .contains("{refs}"));
        assert!(parse("claude('p', {A1:zz})")
            .unwrap_err()
            .contains("bad range"));
    }

    #[test]
    fn task_text_interpolates_prose_and_context() {
        let call = parse("claude('explain {failed} failures', {A1:A2})").unwrap();
        let lookup = |name: &str, _: Form| match name {
            "failed" => "3".to_string(),
            "A1" => "x".to_string(),
            "A2" => "y".to_string(),
            _ => String::new(),
        };
        assert_eq!(
            task_text(&call, &lookup),
            "explain 3 failures\n\nGrid context:\nA1 = x\nA2 = y\n"
        );
        // No context args → no context block.
        let bare = parse("claude('hi')").unwrap();
        assert_eq!(task_text(&bare, &lookup), "hi");
        // A range in the prose reads as rows, and its cells are inputs.
        let lookup = |name: &str, _: Form| match name {
            "A1" => "x".to_string(),
            "B1" => "1".to_string(),
            "A2" => "y".to_string(),
            _ => String::new(),
        };
        let block = parse("claude('compare:\n{A1:B2}')").unwrap();
        assert_eq!(task_text(&block, &lookup), "compare:\nx | 1\ny | ");
        assert_eq!(
            deps("claude('compare {A1:B2}')").unwrap(),
            ["A1", "B1", "A2", "B2"]
        );
    }

    fn setup<'a>(
        agents: &'a HashMap<String, String>,
        paths: &'a HashMap<String, String>,
    ) -> Setup<'a> {
        let models: &'a HashMap<String, String> = Box::leak(Box::default());
        Setup {
            agents,
            paths,
            models,
            resume: models,
            dirs: &[],
        }
    }

    #[test]
    fn command_builds_from_builtin_or_override() {
        let none = HashMap::new();
        let call = parse("claude('do it')").unwrap();
        let cmd = command(
            &call,
            setup(&none, &none),
            "do it",
            "/b/.recalc/mcp.json",
            "B2",
            1,
        )
        .unwrap();
        assert!(
            cmd.starts_with(
                "claude -p --input-format stream-json --output-format stream-json --verbose --permission-prompt-tool stdio --mcp-config '/b/.recalc/mcp.json' --strict-mcp-config"
            ),
            "{cmd}"
        );

        let mut overrides = HashMap::new();
        overrides.insert("claude".to_string(), "fake {task}".to_string());
        let cmd = command(&call, setup(&overrides, &none), "do it", "/m", "B2", 1).unwrap();
        assert_eq!(cmd, "fake 'do it'");

        let call = parse("nope('x')").unwrap();
        let err = command(&call, setup(&none, &none), "x", "/m", "B2", 1).unwrap_err();
        assert!(err.contains("unknown agent") && err.contains("[agents]"));
    }

    #[test]
    fn a_call_names_its_model_and_the_command_carries_it() {
        let call = parse("claude('summarize {A1}', {B1}, model='haiku')").unwrap();
        assert_eq!(call.prompt, "summarize {A1}");
        assert_eq!(call.context, ["B1"]);
        assert_eq!(call.model.as_deref(), Some("haiku"));
        // An apostrophe in the prose still belongs to the prompt.
        let call2 =
            parse("claude(\"it's the report's total\", model=\"claude-opus-5-5\")").unwrap();
        assert_eq!(call2.prompt, "it's the report's total");
        assert_eq!(call2.model.as_deref(), Some("claude-opus-5-5"));
        for bad in [
            "claude('x', model=haiku)",
            "claude('x', model='a b')",
            "claude('x', model='a', model='b')",
            "claude('x', model='')",
            "claude('x', model='h', {A1})",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }

        let none = HashMap::new();
        let cmd = command(&call, setup(&none, &none), "t", "/m", "B2", 1).unwrap();
        assert!(
            cmd.starts_with("claude -p --model 'haiku' --input-format stream-json"),
            "{cmd}"
        );
        let codex = parse("codex('x', model='gpt-5.6-luna')").unwrap();
        let cmd = command(&codex, setup(&none, &none), "t", "/m", "B2", 1).unwrap();
        assert!(
            cmd.starts_with("codex exec --model 'gpt-5.6-luna' --skip-git-repo-check"),
            "{cmd}"
        );
        let sess = session_command(&call, setup(&none, &none), "t", "/m", "s-1").unwrap();
        assert!(sess.contains("--model 'haiku' --session-id"), "{sess}");

        // [models] aliases resolve; an id passes through.
        let mut models = HashMap::new();
        models.insert("haiku".to_string(), "claude-haiku-5-5".to_string());
        let aliased = Setup {
            agents: &none,
            paths: &none,
            models: &models,
            resume: &none,
            dirs: &[],
        };
        let cmd = command(&call, aliased, "t", "/m", "B2", 1).unwrap();
        assert!(cmd.contains("--model 'claude-haiku-5-5'"), "{cmd}");

        // A custom template takes a model only where it says {model}.
        let mut agents = HashMap::new();
        agents.insert("mine".to_string(), "mine -p {task}".to_string());
        agents.insert("wired".to_string(), "wired {model} -p {task}".to_string());
        let err = command(
            &parse("mine('x', model='fast')").unwrap(),
            setup(&agents, &none),
            "t",
            "/m",
            "B2",
            1,
        )
        .unwrap_err();
        assert!(err.contains("has no {model}"), "{err}");
        let cmd = command(
            &parse("wired('x', model='fast')").unwrap(),
            setup(&agents, &none),
            "t",
            "/m",
            "B2",
            1,
        )
        .unwrap();
        assert_eq!(cmd, "wired --model 'fast' -p 't'");
        let cmd = command(
            &parse("wired('x')").unwrap(),
            setup(&agents, &none),
            "t",
            "/m",
            "B2",
            1,
        )
        .unwrap();
        assert_eq!(cmd, "wired -p 't'", "no model, no flag");
    }

    #[test]
    fn the_model_that_ran_is_read_from_claudes_result() {
        let ok = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/agents/claude-2.1.280/ok.json"
        ))
        .unwrap();
        let parsed = parse_output(&ok).unwrap();
        assert_eq!(
            parsed.usage.unwrap().model.as_deref(),
            Some("claude-opus-5-5")
        );
        // The model that cost the most is the one that did the work.
        let two = r#"{"type":"result","is_error":false,"result":"x","total_cost_usd":0.2,"usage":{"input_tokens":1,"output_tokens":1},"modelUsage":{"claude-haiku-5-5":{"costUSD":0.01},"claude-sonnet-5-5[1m]":{"costUSD":0.19}}}"#;
        assert_eq!(
            parse_output(two).unwrap().usage.unwrap().model.as_deref(),
            Some("claude-sonnet-5-5")
        );
    }

    #[test]
    fn paths_relocate_builtins_but_not_custom_templates() {
        let none = HashMap::new();
        let mut paths = HashMap::new();
        paths.insert("claude".to_string(), "/opt/tools/my claude".to_string());
        paths.insert("codex".to_string(), "/opt/codex".to_string());
        let call = parse("claude('go')").unwrap();
        let cmd = command(&call, setup(&none, &paths), "go", "/m", "B2", 1).unwrap();
        assert!(
            cmd.starts_with("'/opt/tools/my claude' -p --input-format stream-json"),
            "{cmd}"
        );
        let cmd = session_command(&call, setup(&none, &paths), "go", "/m", "sid").unwrap();
        assert!(cmd.starts_with("'/opt/tools/my claude' 'go'"), "{cmd}");
        let cmd = resume_command(&call, setup(&none, &paths), "sid", "/m").unwrap();
        assert_eq!(
            cmd,
            "'/opt/tools/my claude' --resume 'sid' --mcp-config '/m'"
        );
        let call = parse("codex('go')").unwrap();
        let cmd = command(&call, setup(&none, &paths), "go", "/m", "B2", 1).unwrap();
        assert!(cmd.starts_with("'/opt/codex' exec"), "{cmd}");
        // A custom template is the user's exact command; paths leave it alone.
        let mut agents = HashMap::new();
        agents.insert("claude".to_string(), "claude-wrapper {task}".to_string());
        let call = parse("claude('go')").unwrap();
        assert_eq!(
            command(&call, setup(&agents, &paths), "go", "/m", "B2", 1).unwrap(),
            "claude-wrapper 'go'"
        );
    }

    #[test]
    fn session_command_is_interactive_and_names_its_conversation() {
        let none = HashMap::new();
        let call = parse("claude('do it')").unwrap();
        let cmd = session_command(&call, setup(&none, &none), "do it", "/m", "1111-2222").unwrap();
        // No -p: the session stays up, under the cell's conversation id.
        assert_eq!(
            cmd,
            "claude 'do it' --session-id '1111-2222' --mcp-config '/m' --strict-mcp-config"
        );
        // Codex picks its own id.
        let call = parse("codex('do it')").unwrap();
        let cmd = session_command(&call, setup(&none, &none), "do it", "/m", "1111-2222").unwrap();
        assert!(!cmd.contains("1111-2222"), "{cmd}");

        // A custom template serves sessions too.
        let mut overrides = HashMap::new();
        overrides.insert("fake".to_string(), "fake {task}".to_string());
        let call = parse("fake('x')").unwrap();
        assert_eq!(
            session_command(&call, setup(&overrides, &none), "x", "/m", "s").unwrap(),
            "fake 'x'"
        );
    }

    #[test]
    fn resume_continues_on_the_calls_model() {
        let none = HashMap::new();
        let claude = parse("claude('x')").unwrap();
        assert_eq!(
            resume_command(&claude, setup(&none, &none), "abc", "/m").unwrap(),
            "claude --resume 'abc' --mcp-config '/m'"
        );
        let haiku = parse("claude('x', model='haiku')").unwrap();
        assert_eq!(
            resume_command(&haiku, setup(&none, &none), "abc", "/m").unwrap(),
            "claude --resume 'abc' --model 'haiku' --mcp-config '/m'"
        );
        let codex = parse("codex('x', model='gpt-5.6-luna')").unwrap();
        let codex = resume_command(&codex, setup(&none, &none), "abc", "/m").unwrap();
        assert!(
            codex.starts_with("codex resume --model 'gpt-5.6-luna' 'abc' "),
            "{codex}"
        );
        assert!(codex.contains("mcp_servers.recalc"), "{codex}");
        let nope = parse("nope('x')").unwrap();
        assert!(resume_command(&nope, setup(&none, &none), "abc", "/m").is_err());
    }

    #[test]
    fn a_custom_template_written_as_the_builtin_talks_as_it_does() {
        let none = HashMap::new();
        let mut agents = HashMap::new();
        let t = |s: &str| s.to_string();
        // Sandboxed, on a model, and still streaming: the prompt on stdin.
        agents.insert(t("streams"), t("cd arena && claude -p {model} --input-format stream-json --output-format stream-json --verbose --permission-prompt-tool stdio --strict-mcp-config"));
        agents.insert(
            t("prints"),
            t("cd arena && claude -p {task} {model} --output-format json"),
        );
        // The prompt as an argument: its output is still just printed.
        agents.insert(
            t("mixed"),
            t("claude -p {task} --output-format stream-json --input-format stream-json"),
        );
        agents.insert(
            t("cx"),
            t("codex exec {model} --json --skip-git-repo-check -"),
        );
        agents.insert(t("local"), t("ollama run gemma {task}"));
        let set = setup(&agents, &none);
        assert_eq!(protocol("streams", set), Protocol::ClaudeStream);
        assert_eq!(protocol("prints", set), Protocol::Plain);
        assert_eq!(protocol("mixed", set), Protocol::Plain);
        assert_eq!(protocol("cx", set), Protocol::CodexEvents);
        assert_eq!(protocol("local", set), Protocol::Plain);
        let builtins = setup(&none, &none);
        assert_eq!(protocol("claude", builtins), Protocol::ClaudeStream);
        assert_eq!(protocol("codex", builtins), Protocol::CodexEvents);
    }

    #[test]
    fn a_custom_template_resumes_as_the_program_it_runs() {
        let none = HashMap::new();
        let mut agents = HashMap::new();
        // Sandboxed in a folder, no board tools: the same folder, no tools.
        agents.insert(
            "contestant".to_string(),
            "cd arena && claude -p {task} {model} --output-format json --strict-mcp-config"
                .to_string(),
        );
        // With the board's tools: the builtin resume, tools and all.
        agents.insert(
            "wired".to_string(),
            "/opt/bin/claude -p {task} --mcp-config {mcp}".to_string(),
        );
        agents.insert(
            "cx".to_string(),
            "cd 'a b' && codex exec {model} {task}".to_string(),
        );
        agents.insert("mine".to_string(), "mine {task}".to_string());
        agents.insert("told".to_string(), "told {task} {model}".to_string());
        let mut resume = HashMap::new();
        resume.insert(
            "told".to_string(),
            "told --again {session} {model}".to_string(),
        );
        let set = Setup {
            agents: &agents,
            paths: &none,
            models: &none,
            resume: &resume,
            dirs: &[],
        };
        let run = |src: &str| resume_command(&parse(src).unwrap(), set, "abc", "/m");
        assert_eq!(
            run("contestant('x', model='haiku')").unwrap(),
            "cd arena && claude --resume 'abc' --model 'haiku'"
        );
        assert_eq!(
            run("wired('x')").unwrap(),
            "/opt/bin/claude --resume 'abc' --mcp-config '/m'"
        );
        assert_eq!(run("cx('x')").unwrap(), "cd 'a b' && codex resume 'abc'");
        assert_eq!(
            run("told('x', model='m1')").unwrap(),
            "told --again 'abc' --model 'm1'"
        );
        // Something else: the error says how to teach recalc.
        assert!(run("mine('x')").unwrap_err().contains("[resume]"));
        assert_eq!(program_of("contestant", set), Some("claude"));
        assert_eq!(program_of("cx", set), Some("codex"));
        assert_eq!(program_of("mine", set), None);
        assert_eq!(program_of("claude", set), Some("claude"));
    }

    #[test]
    fn driver_command_takes_the_role_as_instructions() {
        let none = HashMap::new();
        let claude = driver_command(
            "claude",
            setup(&none, &none),
            "/m",
            DRIVER_PROMPT,
            "/p/driver.md",
            "sid",
        )
        .unwrap();
        assert_eq!(
            claude,
            "claude --session-id 'sid' --mcp-config '/m' --append-system-prompt-file '/p/driver.md'"
        );
        let codex = driver_command(
            "codex",
            setup(&none, &none),
            "/m",
            DRIVER_PROMPT,
            "/p/driver.md",
            "sid",
        )
        .unwrap();
        assert!(codex.contains("\"--id\", \"driver\""), "{codex}");
        assert!(
            codex.contains("-c 'developer_instructions=\"You are the driver"),
            "codex gets the role as developer instructions: {codex}"
        );
        assert!(
            codex.trim_end().ends_with("\"'"),
            "and no first message after it: {codex}"
        );
        // `--skip-git-repo-check` is `codex exec` only; the TUI exits on it.
        assert!(!codex.contains("--skip-git-repo-check"), "{codex}");
        // A custom driver template is used as-is, the role as its task.
        let mut custom = HashMap::new();
        custom.insert("mine".to_string(), "mine --sys {task}".to_string());
        let mine = driver_command(
            "mine",
            setup(&custom, &none),
            "/m",
            DRIVER_PROMPT,
            "/p",
            "s",
        )
        .unwrap();
        assert!(mine.starts_with("mine --sys 'You are the driver"), "{mine}");
        assert!(
            driver_command("nope", setup(&none, &none), "/m", DRIVER_PROMPT, "/p", "s")
                .unwrap_err()
                .contains("driver =")
        );
    }

    /// Real CLI output recorded by `scripts/record-agent-fixtures.sh`, one
    /// directory per CLI version. Record new versions beside the old ones.
    fn fixture(path: &str) -> String {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/agents");
        std::fs::read_to_string(root.join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    #[test]
    fn recorded_claude_output_parses() {
        for v in ["claude-2.1.280", "claude-2.1.314", "claude-2.1.319"] {
            let ok = parse_output(&fixture(&format!("{v}/ok.json"))).unwrap();
            assert_eq!(ok.text, "pong", "{v}");
            assert!(ok.session_id.is_some());
            let usage = ok.usage.unwrap();
            assert!(usage.cost_usd.unwrap() > 0.0 && usage.input_tokens > 0);

            let bad = parse_output(&fixture(&format!("{v}/bad-model.json"))).unwrap_err();
            assert!(
                bad.message.contains("issue with the selected model"),
                "{v}: {}",
                bad.message
            );
            assert!(!bad.rate_limited);
        }

        // Same shape as the recorded error, with a 429 and no text.
        let limit = parse_output(&fixture("claude-2.1.280/rate-limit.synthetic.json")).unwrap_err();
        assert!(limit.rate_limited);
        assert!(limit.message.contains("usage limit"));
    }

    #[test]
    fn recorded_codex_output_parses() {
        for v in ["codex-0.154.0", "codex-0.163.0", "codex-0.166.0"] {
            let ok = parse_output(&fixture(&format!("{v}/ok.jsonl"))).unwrap();
            assert_eq!(ok.text, "pong", "{v}");
            assert!(ok.session_id.is_some());
            assert!(ok.usage.unwrap().input_tokens > 0);

            let bad = parse_output(&fixture(&format!("{v}/bad-model.jsonl"))).unwrap_err();
            assert_eq!(
                bad.message,
                "The 'nonexistent-model-xyz' model is not supported when using Codex with a ChatGPT account.",
                "{v}: the API's message, not the JSON it came wrapped in"
            );
        }
    }

    /// pi's `--mode json` output, recorded against a scripted model. The
    /// system prompt's lines (every tool's schema) are left out of the record.
    #[test]
    fn recorded_pi_output_parses() {
        let v = "pi-1.5.0";
        let ok = parse_output(&fixture(&format!("{v}/ok.jsonl"))).unwrap();
        assert_eq!(ok.text, "pong");
        assert!(ok.session_id.is_some());
        let usage = ok.usage.unwrap();
        assert_eq!(usage.turns, Some(1));
        assert_eq!(usage.model.as_deref(), Some("m"));
        assert!(usage.input_tokens > 0 && usage.cost_usd.unwrap() > 0.0);

        // A recalc tool call, then the answer: usage is summed over both.
        let tools = parse_output(&fixture(&format!("{v}/tools.jsonl"))).unwrap();
        assert!(tools.text.starts_with("A1 says hello"), "{}", tools.text);
        let usage = tools.usage.unwrap();
        assert_eq!(usage.turns, Some(2));
        assert_eq!(usage.input_tokens, 2000);

        // pi exits 0 and reports the failure in its last message.
        let limit = parse_output(&fixture(&format!("{v}/rate-limit.jsonl"))).unwrap_err();
        assert!(limit.rate_limited);
        assert_eq!(
            limit.message, "Rate limit reached for requests",
            "the API's message, not the JSON it came wrapped in"
        );
    }

    /// A small local model on pi once ended with an empty message; the cell
    /// showed pi's raw events as its value.
    #[test]
    fn a_run_that_ends_without_an_answer_says_so() {
        let pi = r#"{"type":"session","version":3,"id":"s-1","cwd":"/b"}
{"type":"message_end","message":{"role":"assistant","content":[],"stopReason":"stop","usage":{"input":10,"output":0,"cost":{"total":0}}}}"#;
        let codex = r#"{"type":"thread.started","thread_id":"t-1"}
{"type":"turn.completed","usage":{"input_tokens":10,"output_tokens":0}}"#;
        for out in [pi, codex] {
            let e = parse_output(out).unwrap_err();
            assert!(e.message.contains("without an answer"), "{}", e.message);
            assert!(!e.rate_limited);
        }
        // Lines that aren't an agent's run are still plain text.
        assert_eq!(
            parse_output(r#"{"type":"note"}"#).unwrap().text,
            r#"{"type":"note"}"#
        );
    }

    #[test]
    fn pi_runs_connected_through_its_extension() {
        let none = HashMap::new();
        let set = setup(&none, &none);
        let call = parse("pi('x', model='sonnet')").unwrap();
        let cmd = command(&call, set, "t", "/b/.recalc/mcp-cell-B2.json", "B2", 1).unwrap();
        assert_eq!(
            cmd,
            "pi --mode json --model 'sonnet' -e '/b/.recalc/pi-recalc.ts' --recalc-mcp '/b/.recalc/mcp-cell-B2.json' --tools read,grep,find,ls,'mcp__recalc__*'"
        );
        assert_eq!(protocol("pi", set), Protocol::PiEvents);
        assert_eq!(program_of("pi", set), Some("pi"));

        // A session takes recalc's id, and resumes by it.
        let sess = session_command(&call, set, "do it", "/b/.recalc/mcp.json", "s-1").unwrap();
        assert_eq!(
            sess,
            "pi --model 'sonnet' --session-id 's-1' -e '/b/.recalc/pi-recalc.ts' --recalc-mcp '/b/.recalc/mcp.json' 'do it'"
        );
        assert!(session_takes_id("pi", set));
        let resume = resume_command(&call, set, "s-1", "/b/.recalc/mcp.json").unwrap();
        assert!(
            resume.starts_with("pi --session 's-1' --model 'sonnet' -e"),
            "{resume}"
        );

        // The driver gets the role as a system-prompt file.
        let drv = driver_command(
            "pi",
            set,
            "/b/.recalc/mcp-driver.json",
            "role",
            "/b/role.md",
            "d-1",
        )
        .unwrap();
        assert!(
            drv.starts_with("pi --session-id 'd-1' -e '/b/.recalc/pi-recalc.ts'"),
            "{drv}"
        );
        assert!(
            drv.ends_with("--append-system-prompt '/b/role.md'"),
            "{drv}"
        );
        assert!(driver_takes_id("pi", set));
        let drv = driver_resume_command("pi", set, "/m.json", "role", "/b/role.md", "d-1").unwrap();
        assert!(drv.starts_with("pi --session 'd-1'"), "{drv}");

        // A custom template written as the builtin talks and resumes as pi.
        let mut agents = HashMap::new();
        agents.insert(
            "local".to_string(),
            "pi --mode json --model ollama/qwen -e {piext} --recalc-mcp {mcp}".to_string(),
        );
        agents.insert("plain".to_string(), "pi -p {task}".to_string());
        let custom = setup(&agents, &none);
        assert_eq!(protocol("local", custom), Protocol::PiEvents);
        assert_eq!(protocol("plain", custom), Protocol::Plain);
        let resume = resume_command(&parse("plain('x')").unwrap(), custom, "s-2", "/m").unwrap();
        assert_eq!(resume, "pi --session 's-2'");

        assert!(footer("B2", "pi", false, true).contains("read, grep, find and ls"));
        assert!(missing_program("pi").contains("pi.dev"));
    }

    #[test]
    fn failures_read_as_what_to_do() {
        assert!(classify(Some(429), "").rate_limited);
        // Claude Code's wording.
        // "resets 4:10am" is the next 4:10 local time, within a day.
        let e = classify(
            None,
            "You've hit your session limit · resets 4:10am (America/Los_Angeles)",
        );
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let at = e.resets_at.expect("the reset time is read");
        assert!(at > now && at <= now + 24 * 3600, "{at} vs {now}");
        assert_eq!(
            resets_today_at("resets 12am"),
            resets_today_at("resets 12:00am")
        );
        assert!(resets_today_at("resets soon").is_none());
        for said in [
            "You've hit your session limit · resets 4:10am (America/Los_Angeles)",
            "You're out of usage credits. Switch to another model, or manage usage credits at claude.ai/admin-settings/usage, to continue.",
            "Your workspace is out of credits. Ask your workspace owner to refill in order to continue.",
        ] {
            let e = classify(None, said);
            assert!(e.rate_limited, "{said}");
            assert_eq!(e.message, said, "a sentence is kept as said");
        }
        let old = classify(None, "Claude AI usage limit reached|1719876000");
        assert!(old.rate_limited);
        assert_eq!(old.resets_at, Some(1719876000));
        assert!(classify(None, "rate_limit").message.contains("usage limit"));
        assert!(classify(Some(401), "").message.contains("isn't signed in"));
        assert!(classify(None, "authentication_failed")
            .message
            .contains("isn't signed in"));
        assert!(classify(None, "billing_error").message.contains("billing"));
        assert!(classify(Some(529), "").message.contains("overloaded"));
        let kept = classify(Some(500), "Something specific broke");
        assert_eq!(kept.message, "Something specific broke");
        assert!(!kept.rate_limited);
    }

    #[test]
    fn every_codex_template_approves_recalcs_tools() {
        // Without it, `codex exec` refuses every recalc tool call.
        for list in [BUILTIN, SESSION_BUILTIN, RESUME_BUILTIN, DRIVER_BUILTIN] {
            let (_, t) = list.iter().find(|(n, _)| *n == "codex").unwrap();
            assert!(
                t.contains("mcp_servers.recalc.default_tools_approval_mode=\"approve\""),
                "{t}"
            );
        }
    }

    #[test]
    fn a_missing_program_is_found_missing_and_named() {
        assert!(program_exists("sh -c true"));
        assert!(
            program_exists("export RECALC_AGENT='B2'; sh -c true"),
            "after the export prefix"
        );
        assert!(program_exists("/bin/sh -c true"));
        assert!(!program_exists(
            "export RECALC_AGENT='B2'; /nonexistent/claude --x"
        ));
        assert!(!program_exists("no-such-program-xyz --flag"));
        let m = missing_program("claude");
        assert!(
            m.contains("Claude Code isn't installed") && m.contains("[paths]"),
            "{m}"
        );
        assert!(missing_program("mine").contains("[agents]"));
    }

    #[test]
    fn the_footer_tells_each_agent_how_it_can_read() {
        assert!(footer("B2", "claude", false, true).contains("Read and Grep tools"));
        let codex = footer("B2", "codex", false, true);
        assert!(!codex.contains("Read and Grep") && codex.contains("your shell"));
        assert!(!footer("B2", "custom", false, true).contains("{read}"));
        // Names the cell and how to cite; table conventions stay out.
        let f = footer("B2", "claude", false, true);
        assert!(
            f.contains("cell B2") && f.contains("cite_cell with name=\"B2\""),
            "{f}"
        );
        assert!(!f.contains("Not found"), "{f}");
    }

    #[test]
    fn a_headless_codex_run_takes_its_prompt_on_stdin() {
        let none = HashMap::new();
        let call = parse("codex('go')").unwrap();
        let cmd = command(&call, setup(&none, &none), "the prompt", "/m", "B2", 1).unwrap();
        assert!(cmd.ends_with(" -"), "{cmd}");
        assert!(
            !cmd.contains("the prompt"),
            "the prompt goes on stdin, not in the command: {cmd}"
        );
        assert_eq!(
            protocol("codex", setup(&none, &none)),
            Protocol::CodexEvents
        );
    }
}
