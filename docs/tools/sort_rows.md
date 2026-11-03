# sort_rows

> Sort a block of rows by one column; cells move whole and references follow them.

## Source
- `src/mcp/structure.rs`: `sort_rows`; the engine side is `Engine::sort_rows`.

## Inputs

| Field | Type | Required | Description |
|---|---|---|---|
| `column` | string | yes | column letter, e.g. `C` |
| `first` | integer | yes | first row, 1-based |
| `last` | integer | yes | last row |
| `descending` | boolean | no | default false |

## Output

`{ok: true}`.

## Flow

1. Rows `first..=last` are ordered by the column's values: numbers before text, text case-insensitively, blanks last; the sort is stable.
2. Each row moves whole, with its sources, marks, citations and history; references into a moved row follow it.
3. Refused while any cell runs. An agent computing a cell can't use it.

## Errors

- `column is a letter such as C`, `sort needs a 1-based row range`, `<cell> is still running`.
