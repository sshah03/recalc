# tally, with beads

A small invoicing library (see the tally README in the claims-ledger example
for the layout) whose work is tracked in beads. `tools/bd` stands in for the
`bd` command over `.beads/standin.json`.

```
python3 tools/bd list --all --json
python3 tools/bd ready --json
python3 -m unittest discover -s tests -t .
```
