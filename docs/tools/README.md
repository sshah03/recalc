# The MCP tools

One page per tool, in the same layout: what it takes, what it returns, what happens in between, and how it fails. The server is `recalc mcp` (`src/mcp.rs`); `AGENTS.md` says when to reach for each.

| Tool | Does |
|---|---|
| [`list_cells`](list_cells.md) | Snapshot of every cell: name, kind, value, state, review mark, provenance. |
| [`get_cell`](get_cell.md) | Full detail for one cell, and a registered read: if the cell changes later, your next result says so. |
| [`set_cell`](set_cell.md) | Set a cell: a literal value, or a formula/command/agent source by its first character. |
| [`set_range`](set_range.md) | Write many cells in one call and one recompute wave: a block from an A1 anchor, or any cells by name (`cells: {name: content}`). |
| [`get_range`](get_range.md) | Read many cells in one call: a block between two A1 corners, or any cells by name (`names: [...]`); every cell read is registered. |
| [`set_answer_type`](set_answer_type.md) | Give a cell the type its answers must take; this is the check behind a question column's header. |
| [`cite_cell`](cite_cell.md) | Attach the passage that supports a cell's value, or clear it. |
| [`mark_cell`](mark_cell.md) | Flag a cell for a human's attention, with a note, or clear a mark. Verifying is a human act. |
| [`trigger_cell`](trigger_cell.md) | Run a `!` action or `@` agent cell (the only way they run), or force-refresh a `$` cell; blocks until the run lands. |
| [`run_range`](run_range.md) | Run every runnable cell in a rectangle, as Run Selection does, and return at once; for filling a table. |
| [`fill_range`](fill_range.md) | Repeat a cell across a block with references shifted, as a person's fill does. |
| [`edit_structure`](edit_structure.md) | Insert or delete rows or columns; references follow. |
| [`sort_rows`](sort_rows.md) | Sort rows by a column; cells move whole. |
| [`rename_sheet`](rename_sheet.md) | Rename a sheet, the first one included; references to it follow. |
| [`duplicate_sheet`](duplicate_sheet.md) | Copy a sheet to a new one as a fresh copy: prompts and formulas, no answers. |
| [`stop_cell`](stop_cell.md) | Stop a run or take it out of the queue. |
| [`set_format`](set_format.md) | Set bold, italic, alignment, number format or fill. |
| [`ask_agent`](ask_agent.md) | Hand a question to another agent (Claude, Codex, or an `[agents]` template) in a cell of its own, and wait for the answer. |
| [`append_cell`](append_cell.md) | Write a value into a series cell so its history records the point. |
| [`history`](history.md) | A cell's value history, oldest first. |
| [`await_cell`](await_cell.md) | Block until a cell satisfies a predicate, or changes; a long poll. |
| [`ask_cell`](ask_cell.md) | Pose a question on a cell and block until a human answers; the question lands in their Needs You list. |
| [`resolve_decision`](resolve_decision.md) | Answer a pending decision programmatically: by option number, option value, or freetext. |
| [`open_session`](open_session.md) | Ask the window to start, or rejoin, an `@` cell's live session and put it in front of the human. |
| [`show_cell`](show_cell.md) | Ask the window to select a cell so the human looks at it. |

`open_session` and `show_cell` are requests to the window on the board, left as files in `.recalc/inbox/` and taken on its next tick; with no window open they wait. Every client also receives the board's `instructions` on initialize (`src/prompts/instructions.md`).

Blocking tools (`await_cell`, `ask_cell`, `trigger_cell`, `ask_agent`) are long-polls: they answer within `timeout_ms` and expect to be re-called. Every successful result may carry a second content block, `{"changed_inputs": [{name, was, now}]}`, naming cells you read that have moved since. Each change is reported once.
