# edit_structure

> Insert or delete rows or columns; the cells beyond move and references follow them.

## Source
- `src/mcp/structure.rs`: `edit_structure`; the engine side is `Engine::insert` and `Engine::delete`.

## Inputs

| Field | Type | Required | Description |
|---|---|---|---|
| `action` | string | yes | `insert` or `delete` |
| `axis` | string | yes | `rows` or `columns` |
| `at` | integer | yes | 1-based row or column number |
| `count` | integer | no | how many; default 1 |

## Output

`{ok: true}`.

## Flow

1. Cells at and beyond `at` move by `count`; every reference to a moved cell is rewritten, and ranges grow or shrink.
2. A reference to a deleted cell becomes `{#REF!}`, so a formula reading it errors and a command or prompt reading it won't run. A range that loses its last row shrinks to what remains; one wholly deleted becomes `{#REF!}`.
3. Refused while any cell runs, since a moved cell would receive another's result. An agent computing a cell can't use it.

## Errors

- `axis is rows or columns`, `action is insert or delete`, `<cell> is still running; stop it before moving rows or columns`.
