//! Headless agent runs that stream events: `@claude` over stream-json
//! (`--permission-prompt-tool stdio`) and `@codex` over `codex exec --json`.
//! Tool use becomes the cell's activity; a `can_use_tool` control request
//! becomes a question on the cell, answered with a `control_response`.
//!
//! The permission-prompt flag is undocumented; the recordings in
//! `tests/fixtures/agents/claude-<version>/stream-*.jsonl` pin its format.

use crate::exec::{self, Capture, Groups, Output, STOPPED};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::oneshot;

/// How long one question may wait on the person.
const ASK_WAIT: Duration = Duration::from_secs(30 * 60);

/// What a run reports while it works.
pub enum Event {
    /// What the agent is doing now, in a few words.
    Activity(String),
    /// A question for the person; the run waits on `reply`.
    Ask(Ask),
}

pub struct Ask {
    pub question: String,
    /// (label, preview) per option.
    pub options: Vec<(String, Option<String>)>,
    /// Whether an answer in the person's own words is taken.
    pub freetext: bool,
    pub reply: oneshot::Sender<Answer>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    Option(usize),
    Text(String),
}

/// Runs a stream-json Claude `command` with `task` as the prompt. stdout is
/// the final `result` event; `timeout` excludes time waiting on a person.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    shell: &str,
    command: &str,
    cwd: &Path,
    env: &HashMap<String, String>,
    timeout: Duration,
    mut cancel: oneshot::Receiver<()>,
    groups: &Groups,
    task: &str,
    on_event: impl Fn(Event),
) -> Result<Output, String> {
    let start = Instant::now();
    let exec::Started {
        mut child,
        pgid,
        member: _member,
    } = exec::spawn(shell, command, cwd, env, groups, true).await?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let stdout = child.stdout.take().expect("stdout is piped");
    let err = Capture::spawn(
        child.stderr.take().expect("stderr is piped"),
        0,
        exec::STDERR_CAP,
    );
    let prompt = json!({"type": "user", "message": {"role": "user", "content": task}});
    if stdin
        .write_all(format!("{prompt}\n").as_bytes())
        .await
        .is_err()
    {
        // It exited before reading; its stderr says why.
    }
    let mut lines = BufReader::new(stdout).lines();
    let mut deadline = Instant::now() + timeout;
    let mut result: Option<Value> = None;
    let mut limit: Option<Value> = None;
    let mut trusted = std::collections::HashSet::new();
    let mut stdin = Some(stdin);
    loop {
        let line = tokio::select! {
            line = lines.next_line() => line,
            _ = tokio::time::sleep_until(deadline.into()) => {
                exec::end_group(&mut child, pgid).await;
                return Err("timeout".into());
            }
            _ = &mut cancel => {
                exec::end_group(&mut child, pgid).await;
                return Err(STOPPED.into());
            }
        };
        let Ok(Some(line)) = line else { break };
        let Ok(event) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        match event["type"].as_str().unwrap_or_default() {
            "assistant" => {
                for part in event["message"]["content"].as_array().into_iter().flatten() {
                    if part["type"] == "tool_use" {
                        on_event(Event::Activity(relative(
                            &describe(&part["name"], &part["input"]),
                            cwd,
                        )));
                    }
                }
            }
            "rate_limit_event" if event["rate_limit_info"]["status"] == "rejected" => {
                limit = Some(event["rate_limit_info"].clone());
            }
            "control_request" => {
                let waited = Instant::now();
                let response = answer(&event, &on_event, &mut cancel, &mut trusted).await;
                deadline += waited.elapsed();
                let Some(response) = response else {
                    exec::end_group(&mut child, pgid).await;
                    return Err(STOPPED.into());
                };
                if let Some(w) = stdin.as_mut() {
                    let _ = w.write_all(format!("{response}\n").as_bytes()).await;
                }
            }
            "result" => {
                result = Some(event);
                // Closing stdin ends the session; Claude exits.
                stdin = None;
            }
            _ => {}
        }
    }
    drop(stdin);
    let status = tokio::select! {
        status = child.wait() => status.map_err(|e| format!("failed to wait for the agent: {e}"))?,
        _ = tokio::time::sleep_until(deadline.into()) => {
            exec::end_group(&mut child, pgid).await;
            return Err("timeout".into());
        }
    };
    let _ = tokio::time::timeout(exec::PIPE_GRACE, err.done()).await;
    let stdout = match (result, limit) {
        // A rejected rate-limit event says when it lifts; the result that
        // follows doesn't always.
        (Some(mut r), Some(l)) => {
            r["rate_limited"] = json!(true);
            if let Some(at) = l["resetsAt"].as_u64() {
                r["rate_limit_resets_at"] = json!(at);
            }
            r.to_string()
        }
        (Some(r), None) => r.to_string(),
        (None, _) => String::new(),
    };
    Ok(Output {
        stdout,
        stderr: err.text("the start"),
        exit_code: status.code().unwrap_or(-1),
        duration_ms: start.elapsed().as_millis() as u64,
    })
}

/// Runs `codex exec --json`, reporting activity as events arrive. Codex can't
/// ask a person mid-run. stdout is the event lines `agent::parse_output` reads.
#[allow(clippy::too_many_arguments)]
pub async fn run_codex(
    shell: &str,
    command: &str,
    cwd: &Path,
    env: &HashMap<String, String>,
    timeout: Duration,
    mut cancel: oneshot::Receiver<()>,
    groups: &Groups,
    task: &str,
    on_event: impl Fn(Event),
) -> Result<Output, String> {
    let start = Instant::now();
    let exec::Started {
        mut child,
        pgid,
        member: _member,
    } = exec::spawn(shell, command, cwd, env, groups, true).await?;
    // The prompt, then EOF: `codex exec -` reads it whole before starting.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(task.as_bytes()).await;
    }
    let stdout = child.stdout.take().expect("stdout is piped");
    let err = Capture::spawn(
        child.stderr.take().expect("stderr is piped"),
        0,
        exec::STDERR_CAP,
    );
    let mut lines = BufReader::new(stdout).lines();
    let deadline = Instant::now() + timeout;
    // Keep only the events the parser reads, not command output.
    let mut kept = Vec::new();
    loop {
        let line = tokio::select! {
            line = lines.next_line() => line,
            _ = tokio::time::sleep_until(deadline.into()) => {
                exec::end_group(&mut child, pgid).await;
                return Err("timeout".into());
            }
            _ = &mut cancel => {
                exec::end_group(&mut child, pgid).await;
                return Err(STOPPED.into());
            }
        };
        let Ok(Some(line)) = line else { break };
        let Ok(event) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let kind = event["type"].as_str().unwrap_or_default();
        if kind == "item.started" {
            if let Some(what) = describe_codex(&event["item"]) {
                on_event(Event::Activity(relative(&what, cwd)));
            }
        }
        let item = event["item"]["type"].as_str().unwrap_or_default();
        if matches!(
            kind,
            "thread.started" | "turn.completed" | "turn.failed" | "error"
        ) || (kind == "item.completed" && item == "agent_message")
        {
            kept.push(line);
        }
    }
    let status = tokio::select! {
        status = child.wait() => status.map_err(|e| format!("failed to wait for the agent: {e}"))?,
        _ = tokio::time::sleep_until(deadline.into()) => {
            exec::end_group(&mut child, pgid).await;
            return Err("timeout".into());
        }
    };
    let _ = tokio::time::timeout(exec::PIPE_GRACE, err.done()).await;
    Ok(Output {
        stdout: kept.join("\n"),
        stderr: err.text("the start"),
        exit_code: status.code().unwrap_or(-1),
        duration_ms: start.elapsed().as_millis() as u64,
    })
}

/// A few words for what a Codex item is starting: `Shell: ls`.
fn describe_codex(item: &Value) -> Option<String> {
    let what = match item["type"].as_str()? {
        "command_execution" => {
            let cmd = item["command"].as_str().unwrap_or_default();
            // Codex runs each command as `<shell> -lc <command>`.
            let inner = cmd.split_once(" -lc ").map_or(cmd, |(_, c)| c);
            let inner = inner
                .strip_prefix('\'')
                .and_then(|c| c.strip_suffix('\''))
                .unwrap_or(inner);
            format!("Shell: {}", first_line(inner))
        }
        "file_change" => {
            let paths: Vec<&str> = item["changes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| c["path"].as_str())
                .collect();
            format!("Edit {}", paths.join(", "))
        }
        "mcp_tool_call" => {
            let tool = item["tool"].as_str().unwrap_or("a tool");
            match item["server"].as_str() {
                Some("recalc") => format!("recalc {tool}"),
                Some(server) => format!("{server} {tool}"),
                None => tool.to_string(),
            }
        }
        "web_search" => format!("Search {}", item["query"].as_str().unwrap_or_default()),
        _ => return None,
    };
    Some(if what.chars().count() > 80 {
        what.chars().take(79).collect::<String>() + "…"
    } else {
        what
    })
}

/// A few words for a tool call: `Bash: cargo test`, `Read src/main.rs`.
fn describe(name: &Value, input: &Value) -> String {
    let name = name.as_str().unwrap_or("a tool");
    let arg = |k: &str| input[k].as_str().map(first_line);
    let short = name
        .strip_prefix("mcp__recalc__")
        .map(|t| format!("recalc {t}"));
    let what = match name {
        "Bash" => arg("command").map(|c| format!("Bash: {c}")),
        "Read" | "Edit" | "Write" | "NotebookEdit" => {
            arg("file_path").map(|p| format!("{name} {p}"))
        }
        "Grep" | "Glob" => arg("pattern").map(|p| format!("{name} {p}")),
        "WebFetch" => arg("url").map(|u| format!("WebFetch {u}")),
        "WebSearch" => arg("query").map(|q| format!("WebSearch {q}")),
        _ => short.map(|t| match input["name"].as_str() {
            Some(cell) => format!("{t} {cell}"),
            None => t,
        }),
    };
    let mut what = what.unwrap_or_else(|| name.to_string());
    if what.chars().count() > 80 {
        what = what.chars().take(79).collect::<String>() + "…";
    }
    what
}

/// Strips the board's path from an activity line; agents use absolute paths.
fn relative(what: &str, cwd: &Path) -> String {
    let base = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    let mut out = what.to_string();
    for b in [base.as_path(), cwd] {
        let prefix = format!("{}/", b.display());
        out = out.replace(&prefix, "");
    }
    out
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or_default().trim().to_string()
}

/// Turns a `control_request` into questions and the answers into a
/// `control_response`; None if the run was stopped while waiting. `trusted`
/// holds the tools the person allowed for the rest of the run.
async fn answer(
    event: &Value,
    on_event: &impl Fn(Event),
    cancel: &mut oneshot::Receiver<()>,
    trusted: &mut std::collections::HashSet<String>,
) -> Option<Value> {
    let id = event["request_id"].clone();
    let request = &event["request"];
    if request["subtype"] != "can_use_tool" {
        return Some(json!({"type": "control_response", "response": {
            "subtype": "error", "request_id": id,
            "error": format!("recalc doesn't handle {} requests", request["subtype"]),
        }}));
    }
    let tool = request["tool_name"].as_str().unwrap_or("a tool");
    let input = &request["input"];
    let decided = if tool == "AskUserQuestion" {
        // Each question in turn; the answers go back keyed by question.
        let mut answers = serde_json::Map::new();
        for q in input["questions"].as_array().into_iter().flatten() {
            let text = q["question"].as_str().unwrap_or_default().to_string();
            let options: Vec<(String, Option<String>)> = q["options"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|o| {
                    (
                        o["label"].as_str().unwrap_or_default().to_string(),
                        o["description"].as_str().map(str::to_string),
                    )
                })
                .collect();
            let labels: Vec<String> = options.iter().map(|o| o.0.clone()).collect();
            let got = ask(on_event, cancel, text.clone(), options, true).await?;
            let chosen = match got {
                Answer::Option(i) => labels
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| "No answer was given.".into()),
                Answer::Text(t) => t,
            };
            answers.insert(text, json!(chosen));
        }
        let mut updated = input.clone();
        updated["answers"] = Value::Object(answers);
        json!({"behavior": "allow", "updatedInput": updated})
    } else if trusted.contains(tool) {
        json!({"behavior": "allow", "updatedInput": input})
    } else {
        let what = describe(&request["tool_name"], input);
        let question = format!("The agent wants to use {what}. Allow it this once?");
        let detail = serde_json::to_string_pretty(input).ok();
        let options = vec![
            ("Allow once".to_string(), detail),
            (
                format!("Allow {tool} for this run"),
                Some(format!(
                    "Don't ask again about {tool} until this run ends; the next run asks afresh"
                )),
            ),
            (
                "Deny".to_string(),
                Some("The agent is told a person turned it down".into()),
            ),
        ];
        match ask(on_event, cancel, question, options, true).await? {
            Answer::Option(0) => json!({"behavior": "allow", "updatedInput": input}),
            Answer::Option(1) => {
                trusted.insert(tool.to_string());
                json!({"behavior": "allow", "updatedInput": input})
            }
            Answer::Option(_) => {
                json!({"behavior": "deny", "message": "A person turned this down in recalc."})
            }
            // Free text is a denial carrying the person's reason.
            Answer::Text(t) => json!({"behavior": "deny", "message": t}),
        }
    };
    Some(json!({"type": "control_response", "response": {
        "subtype": "success", "request_id": id, "response": decided,
    }}))
}

/// Asks the person one question and waits. A dropped reply is a refusal;
/// a stop is None.
async fn ask(
    on_event: &impl Fn(Event),
    cancel: &mut oneshot::Receiver<()>,
    question: String,
    options: Vec<(String, Option<String>)>,
    freetext: bool,
) -> Option<Answer> {
    let (tx, rx) = oneshot::channel();
    on_event(Event::Ask(Ask {
        question,
        options,
        freetext,
        reply: tx,
    }));
    tokio::select! {
        got = rx => Some(got.unwrap_or(Answer::Option(usize::MAX))),
        _ = cancel => None,
        // The run's timeout is paused while waiting, so the wait needs its own.
        _ = tokio::time::sleep(ASK_WAIT) => Some(Answer::Text(format!(
            "Nobody answered within {} minutes; carry on without it.",
            ASK_WAIT.as_secs() / 60
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// The Claude Code versions with recorded stream-json sessions, oldest first.
    const CLAUDE: [&str; 2] = ["claude-2.1.280", "claude-2.1.314"];

    /// A stand-in `claude` that replays a recording (a path under
    /// tests/fixtures/agents), reading a response after each
    /// `control_request`. What it reads is logged to `stdin.log`.
    fn replay(recording: &str, dir: &Path) -> String {
        let src = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/agents")
            .join(recording);
        let script = dir.join("fake-claude.sh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nlog={log}\nread -r line; echo \"$line\" >> \"$log\"\nwhile IFS= read -r ev <&3; do\n  printf '%s\\n' \"$ev\"\n  case \"$ev\" in *'\"control_request\"'*) read -r line; echo \"$line\" >> \"$log\";; esac\ndone 3< {src}\n",
                log = dir.join("stdin.log").display(),
                src = src.display()
            ),
        )
        .unwrap();
        format!("sh {}", script.display())
    }

    async fn play(recording: &str, reply: Answer) -> (Output, Vec<String>, Vec<String>, String) {
        let dir = tempfile::tempdir().unwrap();
        let cmd = replay(recording, dir.path());
        let seen = Arc::new(Mutex::new((Vec::new(), Vec::new())));
        let s = seen.clone();
        let out = run(
            "sh",
            &cmd,
            dir.path(),
            &Default::default(),
            Duration::from_secs(20),
            oneshot::channel().1,
            &Groups::default(),
            "do the thing",
            move |e| match e {
                Event::Activity(a) => s.lock().unwrap().0.push(a),
                Event::Ask(ask) => {
                    s.lock().unwrap().1.push(ask.question.clone());
                    let _ = ask.reply.send(reply.clone());
                }
            },
        )
        .await
        .unwrap();
        let stdin = std::fs::read_to_string(dir.path().join("stdin.log")).unwrap();
        let (acts, asks) = seen.lock().unwrap().clone();
        (out, acts, asks, stdin)
    }

    #[tokio::test]
    async fn a_permission_request_becomes_a_question_and_the_answer_goes_back() {
        for v in CLAUDE {
            let (out, acts, asks, stdin) = play(
                &format!("{v}/stream-permission-allowed.jsonl"),
                Answer::Option(0),
            )
            .await;
            assert_eq!(acts, ["Bash: mkdir probe-dir"], "{v}");
            assert_eq!(
                asks,
                ["The agent wants to use Bash: mkdir probe-dir. Allow it this once?"],
                "{v}"
            );
            let sent: Vec<Value> = stdin
                .lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect();
            assert_eq!(sent[0]["message"]["content"], "do the thing");
            assert_eq!(sent[1]["response"]["response"]["behavior"], "allow");
            assert_eq!(
                sent[1]["response"]["response"]["updatedInput"]["command"],
                "mkdir probe-dir"
            );
            let parsed = crate::agent::parse_output(&out.stdout).unwrap();
            assert_eq!(parsed.text, "done", "{v}");
        }
    }

    #[tokio::test]
    async fn allowing_a_tool_for_the_run_answers_its_next_request_without_asking() {
        let (_, _, asks, stdin) = play(
            "claude-2.1.280/stream-permission-twice.synthetic.jsonl",
            Answer::Option(1),
        )
        .await;
        assert_eq!(asks.len(), 1, "asked again: {asks:?}");
        let sent: Vec<Value> = stdin
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(sent[1]["response"]["response"]["behavior"], "allow");
        assert_eq!(sent[2]["response"]["response"]["behavior"], "allow");
        assert_eq!(
            sent[2]["response"]["response"]["updatedInput"]["command"],
            "mkdir probe-dir-2"
        );
    }

    #[tokio::test]
    async fn a_refusal_goes_back_as_a_denial() {
        for v in CLAUDE {
            let recording = format!("{v}/stream-permission-denied.jsonl");
            let (_, _, _, stdin) = play(&recording, Answer::Option(2)).await;
            let sent: Value = serde_json::from_str(stdin.lines().nth(1).unwrap()).unwrap();
            assert_eq!(sent["response"]["response"]["behavior"], "deny", "{v}");
            let (_, _, _, stdin) = play(&recording, Answer::Text("use a temp dir".into())).await;
            let sent: Value = serde_json::from_str(stdin.lines().nth(1).unwrap()).unwrap();
            assert_eq!(
                sent["response"]["response"]["message"], "use a temp dir",
                "{v}"
            );
        }
    }

    #[tokio::test]
    async fn ask_user_question_reaches_the_person() {
        for v in CLAUDE {
            let (out, _, asks, stdin) =
                play(&format!("{v}/stream-ask.jsonl"), Answer::Option(0)).await;
            assert_eq!(asks, ["Do you prefer tea or coffee?"], "{v}");
            let sent: Value = serde_json::from_str(stdin.lines().nth(1).unwrap()).unwrap();
            assert_eq!(
                sent["response"]["response"]["updatedInput"]["answers"]
                    ["Do you prefer tea or coffee?"],
                "Tea"
            );
            let text = crate::agent::parse_output(&out.stdout).unwrap().text;
            assert_eq!(text.trim_end_matches('.'), "Tea", "{v}");
        }
    }

    #[tokio::test]
    async fn a_codex_run_reports_its_commands_and_keeps_its_answer() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/agents/codex-0.154.0/tools.jsonl");
        let seen = std::sync::Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        let out = run_codex(
            "sh",
            &format!("cat {}", src.display()),
            &std::env::temp_dir(),
            &Default::default(),
            Duration::from_secs(20),
            oneshot::channel().1,
            &Groups::default(),
            "the task",
            move |e| {
                if let Event::Activity(a) = e {
                    s.lock().unwrap().push(a)
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(
            *seen.lock().unwrap(),
            ["Shell: ls", "Shell: cat one.txt", "Shell: wc -l < one.txt"]
        );
        let parsed = crate::agent::parse_output(&out.stdout).unwrap();
        assert_eq!(parsed.text, "2", "the last message is the answer");
        assert!(parsed.session_id.is_some());
        assert!(
            !out.stdout.contains("command_execution"),
            "command items aren't kept"
        );
    }

    #[test]
    fn activity_paths_are_relative_to_the_board() {
        let dir = tempfile::tempdir().unwrap();
        let abs = dir.path().canonicalize().unwrap();
        let line = format!("Read {}/contracts/a.md", abs.display());
        assert_eq!(relative(&line, dir.path()), "Read contracts/a.md");
        assert_eq!(relative("Bash: ls", dir.path()), "Bash: ls");
    }

    #[test]
    fn tool_calls_read_as_a_few_words() {
        assert_eq!(
            describe(&json!("Read"), &json!({"file_path": "src/a.rs"})),
            "Read src/a.rs"
        );
        assert_eq!(
            describe(&json!("mcp__recalc__set_cell"), &json!({"name": "B2"})),
            "recalc set_cell B2"
        );
        assert_eq!(
            describe(&json!("Bash"), &json!({"command": "a\nb"})),
            "Bash: a"
        );
        assert!(describe(&json!("Bash"), &json!({"command": "x".repeat(200)})).ends_with('…'));
    }
}
