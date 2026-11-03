# fill_range

> Repeat the first cell of each row or column across a block, shifting A1 references as a person's fill does.

## Source
- `src/mcp/structure.rs`: `fill_range`; the engine side is `Engine::fill`, the same call ⌘D and ⌘R make.

## Inputs

| Field | Type | Required | Description |
|---|---|---|---|
| `from` | string | yes | top-left corner, e.g. `B2` |
| `to` | string | yes | bottom-right corner, e.g. `L6` |
| `direction` | string | no | `down` or `right`; default `down`, or `right` for a single-row block |

## Output

`{ok: true, filled: n}`.

## Flow

1. Each lane (a column when filling down, a row when filling right) is filled from its first cell. A one-row block filled down takes the cell above; a one-column block filled right takes the cell to the left.
2. Unanchored A1 references shift with the position; a `$` anchor holds its axis (`{$A2}` keeps column A, `{B$1}` keeps row 1). That is how one prompt in B2 becomes a whole document grid.
3. An agent computing a cell can't use it.

## Errors

- `fill needs an A1 range`, `nothing above or left of the selection to fill from`, `direction is down or right`.
