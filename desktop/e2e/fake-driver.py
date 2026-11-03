#!/usr/bin/env python3
"""Stand-in driver: runs in the seat's PTY under tmux, works the board through
`recalc mcp --id driver`, then echoes stdin until the seat ends. Configure:

    driver = "fakedrv"
    [agents]
    fakedrv = "python3 /path/fake-driver.py --mcp-config {mcp}"

It writes plan cells and a small table with set_range, writes a `!` action
and tries to trigger it (refused until a person runs it), and records every
tool result in `driver_report`.
"""
import json
import subprocess
import sys

args = sys.argv[1:]
mcp_config = args[args.index("--mcp-config") + 1]
cfg = json.load(open(mcp_config))["mcpServers"]["recalc"]
server = subprocess.Popen([cfg["command"]] + cfg.get("args", []), stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1)
n = 0
results = []


def rpc(method, params):
    global n
    n += 1
    server.stdin.write(json.dumps({"jsonrpc": "2.0", "id": n, "method": method, "params": params}) + "\n")
    server.stdin.flush()
    while True:
        line = server.stdout.readline()
        if not line:
            raise SystemExit("server closed")
        msg = json.loads(line)
        if msg.get("id") == n:
            return msg


def tool(name, arguments):
    msg = rpc("tools/call", {"name": name, "arguments": arguments})
    r = msg.get("result", {})
    texts = [c.get("text", "") for c in r.get("content", []) if c.get("type") == "text"]
    entry = {"tool": name, "args": arguments, "isError": r.get("isError", False), "text": texts[0] if texts else json.dumps(msg.get("error"))}
    results.append(entry)
    return entry


init = rpc("initialize", {"protocolVersion": "2025-06-18", "clientInfo": {"name": "fake-driver"}, "capabilities": {}})
server.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
server.stdin.flush()
print("fake driver seated; instructions:", len(init.get("result", {}).get("instructions", "")), "chars", flush=True)

tool("set_range", {"cells": {"plan_1": "Lay out the table", "plan_1_status": "todo", "plan_2": "Set up the commit", "plan_2_status": "todo"}})
tool("set_range", {"cells": {"plan_1_status": "done"}})
tool("set_range", {"from": "A1", "values": [["Document", "Question"], ["docs/a.md", "= len({A2})"], ["docs/b.md", "= len({A3})"]]})
tool("set_cell", {"name": "C2", "content": "= concat({$A2}, '/', {B$1})"})
tool("fill_range", {"from": "C2", "to": "C3"})
tool("edit_structure", {"action": "insert", "axis": "rows", "at": 2})
tool("set_cell", {"name": "commit_cmd", "content": "! echo committed > committed.txt"})
tool("trigger_cell", {"name": "commit_cmd"})
tool("get_range", {"names": ["plan_1", "plan_1_status", "plan_2_status"]})
tool("set_cell", {"name": "driver_report", "content": json.dumps(results)})
print("fake driver: board laid out", flush=True)

for line in sys.stdin:
    line = line.strip()
    if line == "exit":
        break
    print(f"fake driver heard: {line}", flush=True)
try:
    server.stdin.close()
except Exception:
    pass
