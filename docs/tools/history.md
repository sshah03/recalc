# history

> A cell's value history, oldest first.

## Source
- `src/mcp.rs`: `tool_list()` declares the schema; the handler is the `"history"` arm of `call_tool` (or its named method).

## Inputs

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | yes | cell name |
| `limit` | integer | no | default 50 |

## Output

`[{at, value}]`, `at` in Unix milliseconds.

## Flow

1. Read from `.recalc/history/<name>.log`; long logs are rotated to the last 500 entries.

## Errors

- None; a cell with no history is an empty array.
