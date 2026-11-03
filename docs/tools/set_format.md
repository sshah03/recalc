# set_format

> Set how cells display: bold, italic, alignment, number format, fill.

## Source
- `src/mcp/structure.rs`: `set_format`; the engine side is `Engine::set_format`.

## Inputs

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | one of | a cell |
| `range` | string | one of | an A1 range, e.g. `C2:C9` |
| `bold`, `italic` | boolean | no | |
| `align` | string | no | `left`, `center`, `right` |
| `num` | string | no | `text`, `integer`, `number`, `currency`, `percent`, `date` |
| `fill` | string | no | `yellow`, `green`, `blue`, `red`, `purple`, `gray` |

## Output

`{ok: true, formatted: n}`.

## Flow

Every named cell gets the same format; giving no fields clears it. The format is display only: values are unchanged. An agent computing a cell may format only its own cell and cells outside its inputs and other agents' answers.

## Errors

- `set_format needs name or range`, an unknown value names the field and the values it takes.
