# open_session

> Put an `@` agent cell's live terminal in front of the human: the window starts the cell's interactive session, or rejoins it, and shows it.

## Source
- `src/mcp.rs`: `tool_list()` declares the schema; the handler is the `"open_session"` arm of `call_tool`, through `Server::request`.
- `src/store.rs`: `request` / `take_requests`; the request is a file in `.recalc/inbox/`.
- `desktop/src-tauri/src/board.rs`: the pump loop takes the inbox and runs `commands::start_session`, then emits `show-session` to the webview.

## Inputs

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | yes | an `@` agent cell |

## Output

`{ok: true, note}`. The note says the request was left for the window; nothing confirms it was seen.

## Flow

1. The cell must exist and be an `@` cell; a headless run (`trigger_cell`) needs no session.
2. The request lands in `.recalc/inbox/` and is taken on the app's next tick. With no window open on the board it waits for one.
3. The window starts the session in tmux when tmux is on PATH (so it outlives the window and can be joined from a terminal), or rejoins a session already there, selects the cell, and opens its terminal in the inspector. `<name>_status` reads `running`.
4. The session is for the human to type in. Steer the worker through the cells it reads, not through its terminal.

## Errors

- `no such cell`, `<name> is not an @ agent cell`.
