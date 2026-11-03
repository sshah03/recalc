# show_cell

> Select a cell in the human's window so they look at it.

## Source
- `src/mcp.rs`: `tool_list()` declares the schema; the handler is the `"show_cell"` arm of `call_tool`, through `Server::request`.
- `src/store.rs`: `request` / `take_requests`; the request is a file in `.recalc/inbox/`.
- `desktop/src-tauri/src/board.rs`: the pump loop takes the inbox and emits `show-cell` to the webview.

## Inputs

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | yes | cell name |

## Output

`{ok: true, note}`. The note says the request was left for the window; nothing confirms it was seen.

## Flow

1. The cell must exist.
2. The request lands in `.recalc/inbox/` and is taken on the app's next tick; with no window open it waits.
3. The window selects the cell: the inspector shows it and the grid scrolls to it. Nothing else changes. Use it for the result you want reviewed, the question you left, or the row that went red. For a decision, use `ask_cell`, which lands in Needs You and blocks you.

## Errors

- `no such cell`.
