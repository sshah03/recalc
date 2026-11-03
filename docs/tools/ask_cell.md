# ask_cell

> Pose a question on a cell and block until a human answers; the question lands in their Needs You list.

## Source
- `src/mcp.rs`: `tool_list()` declares the schema; the handler is the `"ask_cell"` arm of `call_tool` (or its named method).

## Inputs

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | yes | the cell the answer becomes the value of |
| `question` | string | yes | what you need decided |
| `options` | (string | {label, value?, preview?})[] | no | 2–5 choices; `preview` is a few lines showing what the choice means |
| `allow_freetext` | boolean | no | default true |
| `recommended` | integer | no | zero-based index of the option you propose; shown as the default, never picked for the human |
| `timeout_ms` | integer | no | default 45000, at most 50000 |
| `for` | string | no | the id of an agent that may answer instead of the person: `driver`, or the cell another agent computes. The person can always answer. |

## Output

`{resolved: true, value, resolved_by}` when answered; `{resolved: false}` on timeout. Call again with the same question to re-attach.

## Flow

1. Ask only when the options have materially different tradeoffs a human should weigh. Check the cells, the files, and the board's conventions first; when several options would do, take the conservative one yourself and say so in a `note`.
2. The cell enters `awaiting`; recomputation leaves it alone until answered.
3. Re-asking the same question re-attaches to the pending decision. A different question on the same cell is an error.
4. A timeout never picks anything, including the recommended option. If the human never answers, make the decision yourself.
5. With `for`, the named agent can answer through `resolve_decision`; Needs You shows the question as `asker → for`. No other agent can.

## Errors

- `cell <name> has a different pending decision: …`.
- `<name> is computed (…); its answer would replace what it computed`: ask on an empty or literal cell, never on a formula, command or agent cell.

## Notes

Put the tradeoff in an option's `preview`, not its label; labels stay short so a digit can pick them.
