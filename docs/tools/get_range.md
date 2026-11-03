# get_range

> Read many cells in one call: a 2-D block between two A1 corners, or any cells by name. Every cell read becomes a registered read.

## Source
- `src/mcp.rs`: `tool_list()` declares the schema; the handler is the `"get_range"` arm of `call_tool` (`get_range`).

## Inputs

Give either two corners or a list of names.

| Field | Type | Required | Description |
|---|---|---|---|
| `from` | string | block form | one corner, e.g. `A1` |
| `to` | string | block form | the other corner, e.g. `C10` |
| `names` | string[] | named form | any cells by name, named (`n_reproduced`) or A1 |

## Output

Block form: `[[value]]`, rows in order, empty cells as `""`. Named form: `{name: value}`, a cell that doesn't exist as `""`.

## Flow

1. Corners may be given in either order.
2. Each cell read is remembered for `changed_inputs`.

## Errors

- A corner that isn't an A1 cell: a named cell can't be a corner, and the error says to use `names` instead.
- `names must be strings`.
