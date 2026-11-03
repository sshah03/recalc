# Working with recalc

This repo has a **recalc** board open: a live grid of cells that you and the human share. Cells hold values, formulas, or shell commands, and they react to each other. Everything is backed by plain files in `.recalc/`, and the human is watching the grid while you work. Treat it as your shared workspace and working memory, and use it throughout the task.

You reach it through these tools: `get_cell`, `set_cell`, `get_range`, `set_range`, `set_answer_type`, `trigger_cell`, `run_range`, `ask_agent`, `append_cell`, `history`, `await_cell`, `ask_cell`, `list_cells`, `resolve_decision`, `mark_cell`, `cite_cell`, `rename_sheet`, `duplicate_sheet`, and two requests to the human's window, `open_session` and `show_cell` (one page each in `docs/tools/`). Cells come in five kinds, set by the first character:

```
42                    a value
= failed == 0         an expression over other cells
$ cargo test 2>&1     a command; its output becomes the value, re-runs when inputs change
! git commit -am {m}  an action with side effects; runs ONLY when explicitly triggered
@claude('task', {a})  an agent call: runs that agent with this board's MCP wired; like !, trigger-only
```

`{name}` inside a command pulls another cell's value in (shell-quoted) and makes it a dependency; `{name:file}` passes the backing file's path for big values. A1 refs take `$` anchors (`{$A2}`, `{B$1}`) that only matter when a cell is filled across a block. `{file:path}` and `{glob:pattern}` reference repo files the same way, and a `$` cell re-runs when they change. Trailing annotations re-run a `$` cell without a trigger: `# watch: src/** tests/*.py` on file changes (globs separated by spaces), `# every: 5000` on a period.

## Habits that make you effective here

**Plan in the grid.** At the start of a task, lay your subtasks down a column and your claimed status beside them. The human should be able to see what you're doing without reading your transcript.

**Put a check next to every claim.** Don't write "tests pass" as a value and move on. Write a `$` cell that runs the tests, and let a formula compare the result to your claim. If you set `claim_done = true`, there should be a `verified = {claim_done} == {status}` cell that will turn red if you're wrong. Prefer making the verification cell green over asserting done in prose.

**Make your checks watch the repo.** A `$` check with `# watch: src/**` (or a `{file:path}` ref) re-runs when the code changes, so your edits update the board automatically. For a check that can't watch, or after changing something outside its watched paths, `trigger_cell` it yourself. A check that shows green on old code misleads the human more than one that shows red.

**Return results as cells.** When your work produces data (a verdict, a score, a list of failures), `set_cell` each named field and keep your final message to a one-line summary. Downstream cells can reference `{verdict}`; nothing can reference your prose.

**Offload durable facts into named cells.** Put the database URL, the flaky test to skip, and the decision you made an hour ago in cells. Your context window will be compacted and you will forget them; the cells persist. After a compaction, re-read the cells before continuing.

**Ask when a human must decide, not when a cell could tell you.** Check the cells, the files, and the board's conventions first; when several options would do, take the conservative one yourself and say so in the `note`. When you reach a real decision point (a destructive step, an ambiguous requirement, a choice with materially different tradeoffs), use `ask_cell`: two to five short options, the tradeoff in an option's `preview` rather than its label, and the one you'd pick as `recommended`. It appears in the human's Needs You list, marked with your recommendation, and blocks you until they answer. Block in short intervals: call `await_cell`/`ask_cell` with a modest timeout (the 45s default is right) and loop until resolved, because a single long block will be killed by your own client's tool timeout. Asking the same question again re-attaches to it instead of creating a duplicate. If the human never answers, make the decision yourself; the board never picks for you, even your recommended option.

**Read suggestions before overwriting.** If the human edits a cell you computed, the edit shows up in `pending_suggestions` on `get_cell` instead of changing the value. Check for suggestions on cells you own and take them seriously, since they are how the human steers you without interrupting. Adopt one with an ordinary `set_cell`.

**Watch for `changed_inputs`.** Cells you read are remembered. When one changes later (the human edited it, another agent wrote it, a check re-ran), your next tool result carries a second block, `{"changed_inputs": [{name, was, now}]}`, once. Re-read those cells before relying on anything you derived from them. Cells you never read don't appear there, so the block only lists changes that affect you.

**Cite what you read.** When a value comes from a document, attach the passage: `cite_cell(name, file, quote, where)` before you answer, or pass `cite` to `set_cell` / `cites` to `set_range`. The file is a path inside the board and the quote is verbatim, so the human can open the document from the cell and see the passage highlighted. Without a citation they have to check the answer by hand. recalc checks that the quote is in the file; if it isn't, the result says `cite_not_found` and the human sees the citation marked, so quote the document's exact words, not a paraphrase. A citation belongs to one answer: recomputing or overwriting the cell clears it, so cite again when you re-answer.

**Respect what the human verified.** A cell with `mark.kind == "verified"` is the human's answer. Your `set_cell` on it doesn't overwrite it; it lands as a suggestion for them. If you disagree, say why in the `note`. You can't change its mark, answer type, or citation either. You can `mark_cell` a cell `flagged` (with a note) when you're unsure of an answer and want a human to look, but only a human can verify. The same goes for questions: `resolve_decision` closes only a question you asked yourself.

**When you're a cell, answer in the form the header asks for.** A document grid's headers say "number only" or "one of W-2, 1099-INT, …" because formulas read your answer next, and typed cells enforce that request as a check (`answer_type` on `get_cell`). A `set_cell` that doesn't fit is refused, saying what it wanted; a run that prints a misfit lands red. Answer again in that form, with no extra words. "Not found" always fits. Put reasoning in an evidence column if there is one, and nowhere else.

**Guard a derived write with `if_unchanged`.** When a value you're about to write was worked out from cells you read, pass `if_unchanged: true` (or the names). If one of them changed in the meantime, the write is refused and the error carries the diff; re-read and answer again instead of writing over a premise that no longer holds. `changed_inputs` tells you your inputs changed; `if_unchanged` stops you from acting on the old values.

**Fill columns with `set_range`, not fifty `set_cell` calls.** It writes the block and recomputes once. It takes named cells too (`cells: {"plan_1": "…", "plan_1_status": "done"}`), and `get_range(names: [...])` reads several named cells in one call. When you're one row of a document grid (a document in column A, questions across row 1), answer in your cell and put the supporting passage where the evidence column asks for it, so the answer can be checked against its source. Give the answer cells an answer type (`set_answer_type` with `range: "C2:C7"`), not the header, and summarize with formulas (`= countif({E2:E7}, 'Yes')`). For work that's easier as a script than as many tool calls, the `recalc` command does from your shell what the tools do (`recalc get B2`, `recalc set B2 '…'`, `recalc list`), on the same board, and writes as you under the same rules (a verified cell takes your write as a suggestion).

## The safety rule

A `$` command may re-run on its own whenever its inputs change, so **`$` is for commands that only read**: tests, greps, status checks. Anything with side effects (commits, deploys, migrations, deletes, pushes) must be a `!` action, which never runs by itself. If you put a destructive command in a `$` cell it will be quarantined and won't auto-run until a human clears it. The first run of an `!` action you write is done by a person: `trigger_cell` refuses it until they have run it once, and again after you change it. When in doubt, make it a `!`. And if you are the agent computing one cell, you can't write or run `$` or `!` cells at all, because you were started without a shell and the board won't give you one.

## How to think about it

The grid is shared reactive memory backed by plain files, and it can do more than the habits above describe. Anything you can express as cells that read, compute, and depend on one another is fair game. If you find yourself repeatedly tracking or re-checking something in your head, make it a cell and let it update itself, so both you and the human can see it.

Most useful patterns turn up while working, so don't treat the habits above as the full set. If you build a board that works well (a way to track a migration, watch a long build, or coordinate a refactor), save it with `recalc save examples/<name>.recalc --note "what it's for"` so the next agent and the humans can find it.

## If you are the driver

One agent may hold the board's **driver** seat: a session the human opens from the window's Driver tab, wired in as `driver`, that works the board on their behalf instead of computing one cell. If that is you, your role text says so. Lay the plan down a column, build tables with `set_range`, start workers by writing `@claude('…')` cells and calling `trigger_cell` on them, or `run_range` on a table's block (they queue under `max_agents`), wait on them with `await_cell`, use `open_session` to put a worker's terminal in front of the human, and use `show_cell` to point at a result. You steer workers through the cells they read, never by typing into their terminals, and you cannot verify.

## If another agent is here too

Others may be working the same grid; each writer is tagged by id. Before you take exclusive ownership of something (a file, a subtask), claim it by writing your id into a lock cell (`lock_authpy`, say), and check such cells before you start. Nothing enforces this; it's a convention that works because everyone can see everyone's locks. Coordinate through cells instead of assuming you're alone.

To get another agent's answer, a second opinion from Codex on Claude's work or the reverse, use `ask_agent`: it writes the question into a cell of its own as an `@codex(…)` or `@claude(…)` call, runs it, and returns the answer, so the handoff is on the board where the human can check it. Agents started by agents stop three deep. When you need another agent to decide something for you, `ask_cell` with `for` set to its id (`driver`, or the cell it computes) lets it answer; the human always can. There's no channel between agents other than cells.
