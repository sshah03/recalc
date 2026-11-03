#!/usr/bin/env python3
"""Record Claude Code's stream-json protocol for the tests in src/stream.rs.

Runs three short headless sessions the way recalc runs an agent cell
(stream-json in and out, permission requests over stdio) and answers the
permission requests the way a person would through Needs You. Costs a few
cents. See docs/RELEASING.md.

The init frame lists the machine's paths, plugins, skills and MCP servers;
it is cut down to the fields the protocol needs before anything is written.
"""
import json
import os
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
INIT_KEEP = ("type", "subtype", "session_id", "model", "permissionMode", "claude_code_version")

SCENARIOS = {
    # A command Claude doesn't treat as read-only: it asks first.
    "stream-permission-allowed": (
        "Create a directory named probe-dir here using the Bash tool (mkdir), then reply with the single word done.",
        lambda req: {"behavior": "allow", "updatedInput": req["input"]},
    ),
    "stream-permission-denied": (
        "Create a directory named probe-dir here using the Bash tool (mkdir), then reply with the single word done.",
        lambda req: {"behavior": "deny", "message": "A person turned this down in recalc."},
    ),
    # A question for the person, answered with its first option.
    "stream-ask": (
        "Use the AskUserQuestion tool to ask me whether I prefer tea or coffee (two options). Then reply with only my answer.",
        lambda req: {
            "behavior": "allow",
            "updatedInput": {
                **req["input"],
                "answers": {req["input"]["questions"][0]["question"]: req["input"]["questions"][0]["options"][0]["label"]},
            },
        },
    ),
}


def scrub(event, home, cwd):
    if event.get("type") == "system" and event.get("subtype") == "init":
        event = {k: event[k] for k in INIT_KEEP if k in event}
    if event.get("type") == "rate_limit_event":
        # The account's usage figures stay out; recalc reads these three.
        info = event.get("rate_limit_info", {})
        event = {"type": "rate_limit_event",
                 "rate_limit_info": {k: info[k] for k in ("status", "resetsAt", "rateLimitType") if k in info}}
    text = json.dumps(event)
    return text.replace(cwd, "/board").replace(home, "/home/user")


def record(name, prompt, answer, out_dir):
    cwd = tempfile.mkdtemp()
    cmd = ["claude", "-p", "--input-format", "stream-json", "--output-format", "stream-json",
           "--verbose", "--permission-prompt-tool", "stdio", "--allowedTools", "Read"]
    p = subprocess.Popen(cmd, cwd=cwd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1)
    p.stdin.write(json.dumps({"type": "user", "message": {"role": "user", "content": prompt}}) + "\n")
    p.stdin.flush()
    lines = []
    for line in p.stdout:
        event = json.loads(line)
        lines.append(scrub(event, os.path.expanduser("~"), os.path.realpath(cwd)))
        if event.get("type") == "control_request":
            response = {"subtype": "success", "request_id": event["request_id"], "response": answer(event["request"])}
            p.stdin.write(json.dumps({"type": "control_response", "response": response}) + "\n")
            p.stdin.flush()
        if event.get("type") == "result":
            break
    p.stdin.close()
    p.wait(timeout=60)
    path = os.path.join(out_dir, name + ".jsonl")
    with open(path, "w") as f:
        f.write("\n".join(lines) + "\n")
    print("recorded", path)


def main():
    version = subprocess.run(["claude", "--version"], capture_output=True, text=True).stdout.split()[0]
    out_dir = os.path.join(ROOT, "tests", "fixtures", "agents", f"claude-{version}")
    os.makedirs(out_dir, exist_ok=True)
    for name in sys.argv[1:] or SCENARIOS:
        prompt, answer = SCENARIOS[name]
        record(name, prompt, answer, out_dir)


if __name__ == "__main__":
    main()
