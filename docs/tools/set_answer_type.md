# set_answer_type

> Give answer cells the type their answers must take; this is the check behind a question column.

## Source
- `src/mcp.rs`: `tool_list()` declares the schema; the handler is the `"set_answer_type"` arm of `call_tool` (or its named method).

## Inputs

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | one of name, range | a cell (an empty one is created), or a document-grid header, whose answers get the answer type |
| `range` | string | one of name, range | the answer cells, like `B2:B7` |
| `kind` | string | yes | `text`, `yesno`, `date`, `number`, `currency`, `choice`, `quote`, `pattern`, `json`, or `none` to clear |
| `options` | string[] | for `choice`, `pattern`; optional for `json` | the allowed answers, as they should be written; for `pattern`, one regex the whole answer must match; for `json`, the top-level keys the object must have |
| `strict` | boolean | no | default true: refuse a misfit; false: land it and note it |
| `not_found` | boolean | no | whether "Not found" (and "n/a", "not stated") fits; default true. In a template spec, a trailing `!` sets it false. |

## Output

`{"ok": true}`, plus `applied_to: [names]` when the answer type went somewhere other than `name`: the cells of a `range`, or the agent cells under a header.

## Flow

1. The answer type is cell meta like a format: it survives value writes and re-installed sources, moves with the cell, and is saved in templates as `@answer_type names… = kind[:opt|opt][~]`.
2. Applies to agent writes (`set_cell`, `set_range`) and to the result of an `@` cell's run. Humans are never held to it.
3. `none` clears it.
4. An answer type holds answers, so one aimed at a document-grid header (a literal in row 1 with `@` cells below it) goes on those `@` cells instead. Agents building a grid tend to type the header, where it would hold nothing.

## Errors

- `answer type must be one of …`, `a choice answer type needs at least two options`, `<name> is verified by a person; an agent can't change its answer type`.

## Notes

The desktop's Add Question Column sets the answer type for every typed column; use this when you build a table yourself.
