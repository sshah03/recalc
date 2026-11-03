# cite_cell

> Attach the passage that supports a cell's value, or clear it.

## Source
- `src/mcp.rs`: `tool_list()` declares the schema; the handler is the `"cite_cell"` arm of `call_tool` (or its named method).

## Inputs

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | yes | cell name |
| `file` | string | with quote | the source: a path inside the board directory, `cell:<name>` for another cell's value (checked against it now), or an http(s) URL (not fetched, so not checked) |
| `quote` | string | with file | the supporting passage, verbatim |
| `where` | string | no | a locator for people: `§7.2`, `page 3` |

## Output

`{"ok": true}`, plus `cite_not_found` when the quote isn't in the file as written: cite again with the document's exact words.

## Flow

1. With only `name`, clears the citation.
2. The value is untouched. A later value write or recompute clears the citation: it backs one answer.
3. The source pane finds the quote in the document with whitespace and curly quotes normalized, so a passage copied from a PDF lands.

## Errors

- `cite.file must be a path inside the board directory`, `cite.quote must be the supporting passage`, `no such cell`, `<name> is verified by a person; an agent can't change its citation`.

## Notes

Cite your own cell before you print your answer; the task text tells you which cell you are.
