# ask_agent

> Hand a question to another agent (Claude, Codex, or an `[agents]` template) in a cell of its own, and wait for the answer.

## Source
- `src/mcp.rs`: `tool_list()` declares the schema; the handler is `ask_agent`.

## Inputs

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | yes | an empty cell for the answer: a named cell like `review_B2`, or an A1 cell |
| `prompt` | string | yes | what to ask. `{refs}` to other cells are filled in and become inputs |
| `agent` | string | no | `claude` (default), `codex`, or a name under `[agents]` in `.recalc/config.toml` |
| `inputs` | string[] | no | more cells or ranges the agent is given as context, e.g. `["B2", "A1:A5"]` |

## Output

What `trigger_cell` returns: `{value, exit_code, stderr, duration_ms}` from the finished run, or `{running: true, state, hint}` if it outlasts a 45 s block, in which case `await_cell` on the cell keeps waiting.

## Flow

1. The call is `set_cell` with `@agent('prompt', {inputs})` as the source, then `trigger_cell`. The handoff is an ordinary agent cell: the person sees who asked what (`def_by` names you), reads and checks the answer, and can edit the prompt and run it again.
2. The prompt is quoted with whichever of `'` and `"` it doesn't contain.
3. The run queues under `max_agents` like any agent cell, and waits out a usage-limit hold.
4. Agents started by agents stop three deep: the MCP server of an agent that deep refuses to start another, and says so. A person can still run the cell.

## Errors

- `ask_agent needs name`, `ask_agent needs a prompt`.
- `unknown agent …`: not a builtin and not under `[agents]`.
- `<name> is verified by a person; ask in an empty cell`.
- `agents here are already 3 deep`.

## Notes

Prefer it to writing and triggering an `@` cell by hand when you want an answer back; write the cell yourself when you want to lay out a table of them first (`set_range`) and run them together.
