# run_range

> Run every `@` agent, `!` action and `$` cell in a rectangle, as the person's Run Selection does, and return at once.

## Source
- `src/mcp.rs`: `tool_list()` declares the schema; the handler is the `"run_range"` arm of `call_tool`. The engine side is `Engine::trigger_many`, the same call Run Selection makes.

## Inputs

| Field | Type | Required | Description |
|---|---|---|---|
| `from` | string | yes | top-left corner, e.g. `B2` |
| `to` | string | yes | bottom-right corner, e.g. `L6` |
| `all` | boolean | no | re-run cells whose answers are current too; default false |

## Output

`{queued: [names], up_to_date: [names], need_a_person: [names], hint}`. It doesn't wait: read the answers with `get_range` as they land, or `await_cell` on one.

## Flow

1. Literals and empty cells in the rectangle are skipped, and so are verified cells and ones already running or queued. Without `all`, so are cells whose answer is current (state ok): only never-run, stale and errored cells run, so re-running a finished table costs only what changed.
2. Cells are triggered in dependency order. A cell that reads another queued or running cell (directly, or through a formula) waits in the queue until that one has landed. A table's "why" column never answers from blanks.
3. At most `max_agents` agent cells run at once; the rest wait their turn.
4. `need_a_person` lists cells the danger scan flags; a person confirms those one at a time. An `!` action an agent wrote is refused until a person has run it once, as with `trigger_cell`.
5. The agent computing a cell (`recalc mcp --cell`) can't use it on a rectangle that holds `$` or `!` cells.

## Errors

- `run_range needs from and to`, `can't write or run $ commands or ! actions`.
