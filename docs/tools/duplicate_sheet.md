# duplicate_sheet

> Copy a sheet to a new one as a fresh copy: prompts and formulas, no answers.

## Source
- `src/mcp/structure.rs`: `duplicate_sheet`; the engine side is `Engine::duplicate_sheet`.

## Inputs

| Field | Type | Required | Description |
|---|---|---|---|
| `from` | string | yes | the sheet to copy, e.g. `Harbor` |
| `to` | string | yes | the new sheet's name, e.g. `Pinecrest` |

## Output

`{ok: true, first, others}`: the board's sheets after the copy, the first one, then the rest in tab order.

## Flow

1. A new sheet named `to` goes in just after `from`.
2. Each of `from`'s cells is written to the same place on the new sheet as written: a formula, command or agent prompt as its source, a value as it is. Formats and answer types come along.
3. Agents' answers, marks and citations don't: an agent cell in the copy hasn't run. Named cells belong to the board and aren't copied.
4. An unqualified reference in the copy (`{B1}`) means the copy's own cell, so changing the copy's inputs (a client folder, say) points everything on it at them. A reference qualified with another sheet's name keeps reading that sheet.

## Errors

- `from and to are sheet names`, `no sheet named …`, a taken or invalid name (from the store). An agent computing a cell can't use it.
