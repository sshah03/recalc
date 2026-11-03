#!/usr/bin/env python3
"""Stand-in `claude` for the free checks: speaks stream-json to recalc and real
JSON-RPC to `recalc mcp`, so a run crosses every layer at no cost.

Run with `-p` (headless), it reads the prompt from stdin. The first line
starting `fake:` scripts the run as `key=value` pairs separated by `;`;
multi-part values use `|`.

  answer=TEXT          the answer (default: ok)
  sleep=SECONDS        wait before answering
  cost=USD             reported cost (default 0.01)
  activity=TEXT        shown as the cell's activity
  get=NAME             get_cell (registers the read)
  getrange=RANGE       get_range (registers the reads)
  sleep2=SECONDS       wait after the reads, before writing
  set=NAME|CONTENT     set_cell
  guard=NAME|CONTENT   set_cell with if_unchanged: true
  cite=FILE|QUOTE      cite a passage for the cell being computed
  mark=NOTE            flag the cell being computed with a note
  ask=NAME|Q|OPT|OPT.. ask_cell on NAME (not a computed cell); the reply is the answer
  perm=TOOL|INPUT      ask permission over the stream (control_request); a deny answers "denied"
  await=NAME|PRED      await_cell until PRED holds (default: != '')
  loop=N               repeat every set= N times
  child=1              leave a background child running (process-group tests)
  hang=1               never answer
  error=TEXT           finish with is_error and this message
  report=NAME          write every tool result to NAME as JSON
  askagent=NAME|PROMPT ask_agent into NAME and wait; the answer reports its reply
  session=NAME         open_session on NAME
  show=NAME            show_cell on NAME
  runrange=FROM|TO     run_range over the block
  settype=RANGE|KIND|STRICT[|OPT|OPT..]  set_answer_type on RANGE
  append=NAME|VALUE    append_cell (repeatable)
  hist=NAME            history of NAME (into the report)
  shell=COMMAND        run COMMAND with sh as the agent (into the report)
  ;;                   ends the directive; the rest of the line is ordinary prompt

Without `-p` (an interactive session) it shows an idle Claude Code screen and
echoes stdin.
"""
import json
import os
import subprocess
import sys
import time
import uuid

args = sys.argv[1:]
mcp_config = args[args.index("--mcp-config") + 1] if "--mcp-config" in args else None


def arg(flag):
    return args[args.index(flag) + 1] if flag in args and args.index(flag) + 1 < len(args) else None


# Conversations, kept as Claude Code keeps them: $CLAUDE_CONFIG_DIR/projects/<folder>/<id>.jsonl.
# Only with CLAUDE_CONFIG_DIR set, so a check opts in and ~/.claude is never touched.
claude_home = os.environ.get("CLAUDE_CONFIG_DIR")


def conversation(sid):
    return os.path.join(claude_home, "projects", os.getcwd().replace("/", "-"), f"{sid}.jsonl")


def keep_conversation(sid):
    if claude_home:
        os.makedirs(os.path.dirname(conversation(sid)), exist_ok=True)
        with open(conversation(sid), "a") as f:
            f.write(json.dumps({"sessionId": sid}) + "\n")


resumed = arg("--resume")
projects = os.path.join(claude_home or "", "projects")
if claude_home and resumed and not any(
    os.path.exists(os.path.join(projects, d, f"{resumed}.jsonl")) for d in (os.listdir(projects) if os.path.isdir(projects) else [])
):
    # What Claude Code does with an id this computer never saw.
    print(f"No conversation found with session ID: {resumed}", flush=True)
    sys.exit(1)
if "-p" not in args:
    if arg("--session-id"):
        keep_conversation(arg("--session-id"))
    # Claude Code's idle screen, so the app reads the session as waiting.
    import shutil
    tui = "\n".join([
        "fake claude session ready",
        f"recalc on PATH: {shutil.which('recalc') or 'missing'}",
        "started as: " + " ".join(a for a in args if a.startswith("--") and a not in ("--mcp-config", "--append-system-prompt-file", "--strict-mcp-config")) + " " + " ".join(args[i + 1] for i, a in enumerate(args) if a in ("--resume", "--session-id") and i + 1 < len(args)),
        "task begins: " + (args[0].splitlines()[0][:100] if args and not args[0].startswith("--") and args[0].strip() else ""),
        "\u273b Done: edited the report.",
        "\u2500" * 40,
        "\u276f ",
        "\u2500" * 40,
        "  \u23f5\u23f5 auto mode on (shift+tab to cycle) \u00b7 \u2190 for agents",
    ])
    print(tui, flush=True)
    for line in sys.stdin:
        print(f"fake session heard: {line.strip()}", flush=True)
        print(tui, flush=True)
    sys.exit(0)
first = sys.stdin.readline()
try:
    prompt = json.loads(first)["message"]["content"]
except Exception:
    prompt = first
if not isinstance(prompt, str):
    prompt = json.dumps(prompt)

# Only the first `fake:` line counts.
script = []
for line in prompt.splitlines():
    if line.strip().startswith("fake:"):
        body = line.strip()[len("fake:"):].split(";;")[0]  # `;;` ends the directive
        for pair in body.split(";"):
            if "=" in pair:
                k, v = pair.split("=", 1)
                script.append((k.strip(), v.strip()))
        break
opts = {}
for k, v in script:
    opts.setdefault(k, []).append(v)
one = lambda k, d=None: opts.get(k, [d])[-1]


def emit(ev):
    sys.stdout.write(json.dumps(ev) + "\n")
    sys.stdout.flush()


session = arg("--resume") or arg("--session-id") or str(uuid.uuid4())
keep_conversation(session)
emit({"type": "system", "subtype": "init", "session_id": session, "model": "fake", "claude_code_version": "fake"})

# ---- MCP client for the server in --mcp-config ----
server = None
own_cell = None
rpc_id = 0
results = []


def mcp_start():
    global server, own_cell
    if not mcp_config:
        return
    cfg = json.load(open(mcp_config))["mcpServers"]["recalc"]
    cmd = [cfg["command"]] + cfg.get("args", [])
    if "--cell" in cmd:
        own_cell = cmd[cmd.index("--cell") + 1]
    server = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1)
    rpc("initialize", {"protocolVersion": "2025-06-18", "clientInfo": {"name": "fake-agent"}, "capabilities": {}})
    server.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
    server.stdin.flush()


def rpc(method, params):
    global rpc_id
    rpc_id += 1
    my = rpc_id
    server.stdin.write(json.dumps({"jsonrpc": "2.0", "id": my, "method": method, "params": params}) + "\n")
    server.stdin.flush()
    while True:
        line = server.stdout.readline()
        if not line:
            raise SystemExit("mcp server closed")
        msg = json.loads(line)
        if msg.get("id") == my:
            return msg


def tool(name, arguments):
    msg = rpc("tools/call", {"name": name, "arguments": arguments})
    entry = {"tool": name, "args": arguments}
    if "error" in msg:
        entry["rpc_error"] = msg["error"]
    else:
        content = msg["result"].get("content", [])
        entry["isError"] = msg["result"].get("isError", False)
        texts = [c.get("text", "") for c in content if c.get("type") == "text"]
        entry["text"] = texts[0] if texts else ""
        for t in texts[1:]:
            try:
                extra = json.loads(t)
                if "changed_inputs" in extra:
                    entry["changed_inputs"] = extra["changed_inputs"]
            except Exception:
                pass
    results.append(entry)
    emit({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "tool_use", "id": "toolu_fake", "name": f"mcp__recalc__{name}", "input": arguments}]}})
    return entry


if one("activity"):
    emit({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "tool_use", "id": "toolu_act", "name": "Bash", "input": {"command": one("activity"), "description": one("activity")}}]}})

if one("child"):
    subprocess.Popen(["sleep", "1000"], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

if one("hang"):
    while True:
        time.sleep(1)

if one("sleep"):
    time.sleep(float(one("sleep")))

answer = one("answer", "ok")

if one("perm"):
    tool_name, _, inp = one("perm").partition("|")
    rid = str(uuid.uuid4())
    emit({"type": "control_request", "request_id": rid, "request": {"subtype": "can_use_tool", "tool_name": tool_name, "display_name": tool_name, "input": {"command": inp, "description": inp}, "tool_use_id": "toolu_perm"}})
    reply = sys.stdin.readline()
    try:
        r = json.loads(reply)
        behavior = r["response"].get("response", {}).get("behavior") or r["response"].get("subtype")
    except Exception:
        behavior = "unparsed"
    results.append({"tool": "control_request", "reply": reply.strip()[:400], "behavior": behavior})
    if behavior == "deny":
        answer = "denied"

needs_mcp = any(k in opts for k in ("get", "getrange", "set", "guard", "cite", "mark", "ask", "await", "report", "askagent", "session", "show", "runrange", "settype", "append", "hist"))
if needs_mcp:
    mcp_start()

for t in opts.get("settype", []):
    parts = t.split("|")
    a = {"range": parts[0], "kind": parts[1], "strict": parts[2].lower() != "false"}
    if len(parts) > 3:
        a["options"] = parts[3:]
    tool("set_answer_type", a)
for name in opts.get("get", []):
    tool("get_cell", {"name": name})
for rng in opts.get("getrange", []):
    tool("get_range", {"range": rng})

if one("sleep2"):
    time.sleep(float(one("sleep2")))

for name in opts.get("await", []):
    cell, _, pred = name.partition("|")
    tool("await_cell", {"name": cell, "predicate": pred or "!= ''", "timeout_ms": 20000})

loops = int(one("loop", "1"))
for _ in range(loops):
    for s in opts.get("set", []):
        cell, _, content = s.partition("|")
        tool("set_cell", {"name": cell, "content": content})
for s in opts.get("guard", []):
    cell, _, content = s.partition("|")
    tool("set_cell", {"name": cell, "content": content, "if_unchanged": True})

for c in opts.get("cite", []):
    file, _, quote = c.partition("|")
    tool("cite_cell", {"name": own_cell or "A1", "file": file, "quote": quote})

if one("mark"):
    tool("mark_cell", {"name": own_cell or "A1", "kind": "flagged", "note": one("mark")})

if one("ask"):
    parts = one("ask").split("|")
    cell, q, options = parts[0], parts[1], parts[2:]
    while True:
        r = tool("ask_cell", {"name": cell, "question": q, "options": [{"label": o} for o in options], "recommended": 0, "timeout_ms": 5000})
        try:
            body = json.loads(r.get("text") or "{}")
        except Exception:
            body = {}
        if body.get("resolved"):
            answer = str(body.get("value"))
            break
        if r.get("isError") or r.get("rpc_error"):
            answer = "ask failed: " + (r.get("text") or json.dumps(r.get("rpc_error")))
            break

for a in opts.get("append", []):
    cell, _, v = a.partition("|")
    tool("append_cell", {"name": cell, "value": v})
for h in opts.get("hist", []):
    tool("history", {"name": h})
for rr in opts.get("runrange", []):
    frm, _, to = rr.partition("|")
    tool("run_range", {"from": frm, "to": to})
for sname in opts.get("show", []):
    tool("show_cell", {"name": sname})
for sname in opts.get("session", []):
    tool("open_session", {"name": sname})
if one("askagent"):
    cell, _, prompt = one("askagent").partition("|")
    r = tool("ask_agent", {"name": cell, "prompt": prompt, "agent": "claude"})
    try:
        body = json.loads(r.get("text") or "{}")
    except Exception:
        body = {}
    while body.get("running"):
        w = tool("await_cell", {"name": cell, "predicate": "!= ''", "timeout_ms": 20000})
        try:
            body = json.loads(w.get("text") or "{}")
        except Exception:
            body = {}
        if body.get("satisfied"):
            break
    answer = "asked " + cell + " -> " + (r.get("text") or json.dumps(r.get("rpc_error")))[:400]

for cmd in opts.get("shell", []):
    p = subprocess.run(["sh", "-c", cmd], capture_output=True, text=True, timeout=30)
    results.append({"tool": "shell", "cmd": cmd, "code": p.returncode, "out": (p.stdout + p.stderr)[:400]})

if one("report"):
    tool("set_cell", {"name": one("report"), "content": json.dumps(results)})

if server:
    # Stop the server as Claude Code does: SIGINT, SIGTERM, kill, stdin left open.
    import signal
    for sig, wait in ((signal.SIGINT, 0.1), (signal.SIGTERM, 1.0)):
        if server.poll() is not None:
            break
        server.send_signal(sig)
        try:
            server.wait(timeout=wait)
        except subprocess.TimeoutExpired:
            pass
    if server.poll() is None:
        server.kill()

if one("error"):
    emit({"type": "result", "subtype": "error", "is_error": True, "result": one("error"), "session_id": session, "total_cost_usd": float(one("cost", "0.01")), "usage": {"input_tokens": 100, "output_tokens": 10}, "num_turns": 1, "duration_ms": 10})
    sys.exit(1)

emit({"type": "result", "subtype": "success", "is_error": False, "result": answer, "session_id": session, "total_cost_usd": float(one("cost", "0.01")), "usage": {"input_tokens": 100, "output_tokens": 10}, "num_turns": 1, "duration_ms": 10})
