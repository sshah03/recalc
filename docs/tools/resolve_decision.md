# resolve_decision

> Answer a pending decision programmatically: by option number, option value, or freetext.

## Source
- `src/mcp.rs`: `tool_list()` declares the schema; the handler is the `"resolve_decision"` arm of `call_tool` (or its named method).

## Inputs

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | yes | cell name |
| `choice` | integer | string | yes | 1-based option number, an option's value or label, or freetext |

## Output

`{"ok": true}`; any waiter on the question resolves too.

## Flow

1. On an agent-posed question the answer becomes the cell's value.
2. On a system-posed edit decision (`suggest` / `pin` / `override` / `cancel`) it selects the action; freetext means 'suggest, with this note'.

## Errors

- `no pending decision on <name>`, `does not allow a freetext answer`.
- `the question on <name> is for a person`: an agent can close only a question it asked itself, or one asked `for` it (`ask_cell`). Another agent's question, a running agent's permission prompt, and the choice recalc puts to a person about their own edit belong to the person.
