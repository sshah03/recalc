# Contributing to recalc

Development setup, the checks, and the layout are in
[desktop/README.md](desktop/README.md). What follows is the part that isn't
about tooling: how a change gets to be complete here.

## The checks

```
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm --prefix desktop/frontend test        # tsc, then the jsdom smoke test
npm --prefix desktop/e2e test             # the real app through its UI: keys, mouse and formulas (needs a debug build)
npm --prefix desktop/e2e run edge         # the app under races, crashes, hostile input and scale, with a stand-in agent
npm --prefix desktop/e2e run boards       # new boards, rename and move, the first run, the command line tool, updates, named cells
```

CI runs these on macOS and Linux, the end-to-end checks on macOS. A change that touches layout or
CSS also needs a person to open the window and look, in both appearances;
jsdom can't see position.

## A bug that reached `main`

Fix it with a test, and show in the pull request that the test fails with
the fix reverted and passes with it applied. A test that was never seen
failing hasn't been shown to guard anything.

## Agent output

What Claude Code and Codex print is recorded under
`tests/fixtures/agents/<cli>-<version>/` and parsed in the tests. When a
feature reads something new from an agent, record it with
`scripts/record-agent-fixtures.sh` or `scripts/record-stream-fixtures.py`
rather than writing the JSON by hand; releasing is in `docs/RELEASING.md`.

## Hit every surface

The most common defect in a project like this is a change that works on
the path you tested and is missing everywhere else. Before calling a change
done, walk this list and say which entries applied:

- **Entry points.** A cell-level behavior is reachable from the native menu
  (`desktop/src-tauri/src/menu.rs`), the command palette and toolbar
  (`desktop/frontend/src/actions.ts`), the right-click menu, the inspector,
  and a keyboard shortcut. A test ties every menu id to an action; the rest
  is on you.
- **Agents.** Anything a human can do to a cell, ask whether an agent can:
  an MCP tool (`src/mcp/`, one page in `docs/tools/`), the CLI
  (`src/cli.rs`), and the raw file contract under `.recalc/`. Decide per
  surface, even if the decision is "not here". Verifying, for one, is
  deliberately human-only.
- **Providers.** Claude Code and Codex differ (output format, pricing, stdin).
  A feature that reads an agent's output needs a decision per builtin
  template and a fallback for a custom one. The builtin `claude` runs over
  stream-json (`src/stream.rs`); Codex and custom templates run through
  `exec::run` and print their answer.
- **Types.** The engine's `Cell` is serialized to the webview; `types.ts`
  mirrors it and a test in `board.rs` fails when they drift. Meta fields
  also need `Meta` in `store.rs`, and the Excel export if they matter to a
  reviewer.
- **Templates.** Cell meta that a board should carry (series, shapes)
  needs a `@line` in `apply_template` and its inverse in `save_template`,
  and the demos and `examples/` should use it.
- **Reverse states.** If you added a way in, add the way out and the way
  to see it: snooze needs wake, and shape needs clear. A state you can
  enter but not leave is a bug.
- **Docs.** `README.md` (short, for newcomers), `docs/GUIDE.md` (everything), `RECIPES.md`, `AGENTS.md` (what agents should do
  with it), and the in-app cheatsheet and guide (`help.ts`).
  Fix what a change makes inaccurate before adding anything.

## Lines to hold

A feature needs a stronger reason than "other tools have it", written down
next to the change.

## Prompts

Text an agent reads lives in `src/prompts/*.md`, not in Rust string
literals, so it can be read and tuned without reading the code around it.
A prompt change ships with a before/after on the demo tables (`recalc demo
legal`, `tax`), because what a prompt change breaks is how consistently
agents answer.

## Commits

Commits are authored by the person making them. One change per commit, in
the imperative, saying what and why in a sentence.
