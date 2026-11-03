# set_range

> Write many cells in one call and one recompute wave, each routed like `set_cell`: a block from an A1 anchor, or any cells by name.

## Source
- `src/mcp.rs`: `tool_list()` declares the schema; the handler is the `"set_range"` arm of `call_tool` (`set_range`, and `set_named` for the named form).

## Inputs

Give either the block form or the named form.

| Field | Type | Required | Description |
|---|---|---|---|
| `from` | string | block form | anchor, e.g. `B2` |
| `values` | string[][] | block form | rows of contents; row 0 lands on the anchor's row |
| `cells` | {name: string} | named form | any cells by name, named (`plan_1`) or A1 (`B2`), and their contents |
| `cites` | ({file, quote, where?} \| null)[][] or {name: cite} | no | mirrors the form used: a citation or null per block cell, or a citation per named cell |
| `if_unchanged` | true \| [names] | no | as on `set_cell` |

## Output

`{"ok": true}`, or an error listing the refused cells after the rest have landed. With `cites`, `cites_not_found: [names]` lists the cells whose quote isn't in its file as written. `made_stale: [names]` lists answered agent cells the write left stale, as on `set_cell`. `errors: {name: reason}` with a `hint` lists sources it stored that can't run as written (a prompt that didn't parse, a reference to a deleted cell; not a formula whose inputs are still empty), as `set_cell`'s `error` does for one cell; they are saved, and running them or opening their sessions is refused until they are set again.

## Flow

1. `if_unchanged` is checked once, before anything is written.
2. Every cell is attempted; refusals (an answer that doesn't fit its answer type, a cycle, a bad name) are collected and reported together, naming each cell. The others land and recompute once, so a formula written alongside the cells it reads sees them.
3. Citations are attached after the values; a verified cell's write lands as a suggestion and its citation is left alone.
4. Sources that landed in an error state are reported under `errors`.

## Errors

- `<cell>: answer doesn't fit its answer type: …; <cell>: …`, with one entry per refused cell.
- A `from` that isn't an A1 cell: a named cell has no block to fill, and the error says to use `cells` instead.
- `values must be an array of arrays`, `cell contents must be strings`.

## Notes

Use the block form to fill a document grid's column, and the named form to lay out or update a plan (`plan_1`, `plan_1_status`, …) or a set of summary cells in one call.
