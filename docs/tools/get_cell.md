# get_cell

> Full detail for one cell, and a registered read: if the cell changes later, your next result says so.

## Source
- `src/mcp.rs`: `tool_list()` declares the schema; the handler is the `"get_cell"` arm of `call_tool` (or its named method).

## Inputs

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | yes | cell name |

## Output

`name`, `kind`, `value`, `source` (the `=`/`$`/`!`/`@` text, or null), `state`, `exit_code`, `stderr`, `duration_ms`, `deps`, `dependents`, `provenance`, `decision` (a pending question, or null), `pending_suggestions` (human proposals you haven't adopted), `mark`, `cite`, `answer_type` (what answers here must look like, or null), `usage` (what the last run cost, or null).

## Flow

1. The value is remembered for this connection. When it differs on a later call, that call's result gains `{"changed_inputs": [{name, was, now}]}` once, and the remembered value moves on.
2. Your own `set_cell` on a read cell re-baselines it: your write is not news to you.

## Errors

- `no such cell: <name>`.

## Notes

Check `answer_type` before answering into a cell: it tells you what will be accepted. Check `pending_suggestions` on cells you own: that is how the human steers you.
