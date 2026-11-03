# acme

A small service about to ship 2.4.0. The release board beside it checks what a
script can, holds what a person must sign, and tags only when everything is.

| Path | What it is |
|---|---|
| `pyproject.toml`, `src/acme/__init__.py` | the two places the version is written |
| `CHANGELOG.md` | one section per release |
| `migrations/` | each `NNN_name.up.sql` needs a `NNN_name.down.sql` |
| `merged.txt` | the pull requests merged since 2.3.0 (what `gh pr list --state merged` would say) |
| `tools/` | stand-ins for `gh` and the tag step, so the board runs without an account |
