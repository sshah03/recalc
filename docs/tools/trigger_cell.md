# trigger_cell

> Run a `!` action or `@` agent cell (the only way they run), or force-refresh a `$` cell; blocks until the run lands.

## Source
- `src/mcp.rs`: `tool_list()` declares the schema; the handler is the `"trigger_cell"` arm of `call_tool` (or its named method).

## Inputs

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | yes | cell name |

## Output

`{value, exit_code, stderr, duration_ms}` from the finished run. The cell becomes a registered read.

A run that outlasts the block returns `{running: true, state, hint}` instead. This is not a failure: the result will land in the cell, and `await_cell` can keep waiting.

## Flow

1. The block is the run's own timeout (`timeout_ms`, or `agent_timeout_ms` for an `@` cell) plus slack, capped at 45s so it returns before the client's tool-call limit.
2. An agent trigger never confirms a quarantined `$` cell; only a human can.
3. An `!` action an agent wrote, or a quarantined `$` an agent wrote, is refused until a person has run it once; after that, until its source changes, agents may re-run it.
4. The agent computing a cell (`recalc mcp --cell`) can't trigger `$` or `!` cells at all.
5. A second trigger while a run is in flight is refused.

## Errors

- `already running`, `not a runnable cell`, `a person runs it the first time`, `can't write or run $ commands or ! actions`.
