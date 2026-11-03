# list_cells

> Snapshot of every cell: name, kind, value, state, review mark, provenance.

## Source
- `src/mcp.rs`: `tool_list()` declares the schema; the handler is the `"list_cells"` arm of `call_tool` (or its named method).

## Inputs

None.

## Output

A JSON array, sorted by name, one object per cell: `name`, `kind`, `value`, `state`, `mark` (`verified` | `flagged` | null), `provenance` (`by`, `at`, `note`).

## Flow

1. Read the engine's mirror; no I/O beyond the pump every call does.
2. Like every successful result, may carry a second content block `{"changed_inputs": […]}` naming cells this connection read that have moved since (see `get_cell`).

## Errors

- None. An empty board is an empty array.

## Notes

Cheap enough to call at the start of a task and after a compaction. It does not register reads: only `get_cell`, `get_range`, and a satisfied `await_cell` do.
