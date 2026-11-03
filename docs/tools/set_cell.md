# set_cell

> Set a cell: a literal value, or a formula/command/agent source by its first character.

## Source
- `src/mcp.rs`: `tool_list()` declares the schema; the handler is the `"set_cell"` arm of `call_tool` (or its named method).

## Inputs

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | yes | cell name |
| `content` | string | yes | a value, or text starting with `=` `$` `!` `@` |
| `note` | string | no | context recorded in provenance |
| `cite` | {file, quote, where?} | no | the passage the value rests on; `file` is a path inside the board, `quote` verbatim |
| `if_unchanged` | true | [names] | no | refuse the write if cells you read have moved since: all of them, or the named ones |

## Output

`{"ok": true}`, plus `made_stale: [names]` with a `hint` when the write left answered agent cells stale (you changed a header or `instructions` they were built from: run them again), `cite_not_found` when a `cite`'s quote isn't in its file as written, and what landed when it differs from what was sent: `written` (and `from`) when the answer type normalized the answer, `outside_answer_type` when a lenient answer type let a misfit through, `unchanged` when the cell already held the value.

## Flow

1. `if_unchanged` is checked first. If a guarded cell moved, the write is refused and the error body is `{refused, changed_inputs: [{name, was, now}]}`; those cells are re-baselined, so re-read them and answer again.
2. A literal written by an agent into a cell with an `answer_type` is checked against it. A fit lands in canonical form (`yes.` → `Yes`, `$1,200` → `1200`, a loosely matched option as listed); `Not found` always fits. A strict answer type refuses a misfit, saying what it wanted; a lenient one lands it and notes the misfit in the activity log.
3. Writing over a cell a human has **verified** does not overwrite it: the content lands in `pending_suggestions` for them.
4. Content starting with `=` `$` `!` `@` installs a source; the engine parses its dependencies, refuses a cycle, and recomputes. `$` runs now if its inputs are ready; `!` and `@` only run on `trigger_cell`.
5. A `cite` is attached after the write; an invalid one (outside the board, empty quote) fails the call after the value has landed.
6. The cell's dependents recompute in one wave.

## Errors

- `<name>: answer doesn't fit its answer type: expected …, got …`: answer again in the form it asks for, with no extra words.
- `{refused: …, changed_inputs: …}`: a guarded input changed.
- `if_unchanged names … you haven't read`: a guard needs a read to compare against, so call `get_cell` first.
- `refusing <name>: cycle: a -> b -> a`.
- `cite.file must be a path inside the board directory`.

## Notes

Prefer `set_range` for a column. Pass `if_unchanged` whenever the value was derived from cells you read.
