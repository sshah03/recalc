# rename_sheet

> Rename a sheet, the first one (Sheet1) included; references to it follow.

## Source
- `src/mcp/structure.rs`: `rename_sheet`; the engine side is `Engine::rename_sheet`.

## Inputs

| Field | Type | Required | Description |
|---|---|---|---|
| `from` | string | yes | the sheet's current name, e.g. `Sheet1` |
| `to` | string | yes | its new name, e.g. `Runs` |

## Output

`{ok: true, first, others}`: the board's sheets after the rename, the first one, then the rest in tab order.

## Flow

1. The sheet's cells keep their places. The first sheet's cells stay unprefixed (`A2`); another sheet's move to the new name (`Runs!A2`).
2. Every reference to the sheet, from any sheet, is rewritten to the new name (`{Sheet1!A2}` becomes `{Runs!A2}`), and the cells that read it recompute.
3. Refused while one of the sheet's cells runs. An agent computing a cell can't use it.

## Errors

- `from and to are sheet names`, a taken or invalid name (from the store), `<cell> is still running`.
