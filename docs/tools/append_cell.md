# append_cell

> Write a value into a series cell so its history records the point.

## Source
- `src/mcp.rs`: `tool_list()` declares the schema; the handler is the `"append_cell"` arm of `call_tool` (or its named method).

## Inputs

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | yes | cell name |
| `value` | string | yes | the point |

## Output

`{"ok": true}`.

## Flow

1. An ordinary literal write; history is kept for every cell, and a series cell draws it as a sparkline.

## Errors

- As `set_cell`.
