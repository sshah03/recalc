# await_cell

> Block until a cell satisfies a predicate, or changes; a long poll.

## Source
- `src/mcp.rs`: `tool_list()` declares the schema; the handler is the `"await_cell"` arm of `call_tool` (or its named method).

## Inputs

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | yes | cell name |
| `predicate` | string | no | a comparison on the value: `!= ''`, `== true`, `> 0.9` |
| `timeout_ms` | integer | no | default 45000 |

## Output

`{value, changed, satisfied}`. `satisfied: false` means the timeout passed first; call again to keep waiting.

## Flow

1. An already-satisfied predicate returns at once.
2. A re-call from the same connection on the same cell replaces the earlier waiter, so a client that killed the previous call leaves no orphan.
3. A satisfied wait registers the cell as read.

## Errors

- An unparseable predicate is refused immediately; an evaluation error (a non-number against `>`) is 'not yet'.
- A bare word is a cell reference (`== done` reads the cell named `done`). A predicate that names a cell that doesn't exist is refused with the quoted form to use instead (`== 'yes'`), since it could otherwise wait forever.

## Notes

Keep timeouts under your client's tool-call limit (~60s for Claude Code) and loop; the server caps a block at 50s.
