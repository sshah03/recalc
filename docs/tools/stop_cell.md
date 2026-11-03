# stop_cell

> Stop a cell's run, or take it out of the queue.

## Source
- `src/mcp/structure.rs`: `stop_cell`; the engine side is `Engine::stop`.

## Inputs

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | yes | the cell |

## Output

`{ok: true, stopped: bool}`; `stopped` is false when nothing was running or queued.

## Flow

The run's process group is killed and its result discarded; a queued cell leaves the queue. The cell keeps its last value and shows as out of date. A pending question from the run is taken down. An agent computing a cell can't use it.
