# mark_cell

> Flag a cell for a human's attention, with a note, or clear a mark. Verifying is a human act.

## Source
- `src/mcp.rs`: `tool_list()` declares the schema; the handler is the `"mark_cell"` arm of `call_tool` (or its named method).

## Inputs

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | yes | cell name |
| `mark` | string | yes | `flagged` | `none` |
| `note` | string | no | why it needs a look |

## Output

`{"ok": true}`.

## Flow

1. A flag appears in the human's Needs You list with your note and how long it has waited.
2. `verified` is refused: only a human can sign off.

## Errors

- `only a human can verify a cell`.
- `<name> is verified by a person; an agent can't change its mark`: an agent can neither clear a human's sign-off nor flag over it. To disagree, write a `set_cell` note, which lands as a suggestion.
