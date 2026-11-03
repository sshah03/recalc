# Recipes

What people actually do with recalc. Each recipe is a loadable template in `examples/` plus a short note on *why* it works. Load one with:

```
recalc open examples/verify.recalctemplate
```

The library is meant to grow from use: when an agent (or you) improvises a board that works, `recalc save examples/<name>.recalc --note "..."` adds it here. In a well-used copy of this repo, some of the best boards were improvised this way instead of designed up front.

## The verification board (`examples/verify.recalctemplate`)

```
raw          $ cargo test 2>&1 # watch: {src,tests}/**
passed       $ grep -oE '[0-9]+ passed' {raw:file} | awk '{s+=$1} END {print s+0}'
failed       $ grep -oE '[0-9]+ failed' {raw:file} | awk '{s+=$1} END {print s+0}'
ignored      $ grep -oE '[0-9]+ ignored' {raw:file} | awk '{s+=$1} END {print s+0}'
status       $ test {raw.code} -eq 0 && echo true || echo false
skips_added  $ git diff HEAD -U0 -- . | grep -cE '^\+.*(#\[ignore\]|\.skip\(|@pytest\.mark\.skip|…)'
baseline     (the test count before the work starts)
tests_kept   = if({baseline} == '', 'no baseline', {total} >= {baseline})
claim_done   (the agent writes "true" here when it believes it's done)
verified     = if({claim_done} == '', 'pending', and({claim_done} == {status}, {skips_added} == 0, {tests_kept} != false))
```

A suite an agent made green by skipping or deleting the failing tests is still green. `skips_added` counts skip markers the working tree adds over the last commit, and `tests_kept` compares the count with `baseline`, written before the work starts (the agent can do it as its first step). Tried on a small Rust crate: an agent's `#[ignore]` on a failing test left `status` true and turned `verified` false.

The board is live: `raw` watches the repo (adapt the glob to your layout), so an edit, yours or the agent's, re-runs the suite. The gate is the suite's exit status, `{raw.code}`, instead of a parsed count, because a workspace with several test binaries prints several summaries, and a count that read only the first could say 0 failed while the second binary was red. The parse cells sum every summary from `raw`'s cached output through `{raw:file}`; they refresh when `raw` does, without a second test run, and multi-kilobyte output never lands on an argv. The cell to watch is `verified`: the agent's *claim* and the actual test result sit side by side, and when the agent says done while the suite is red, `verified` turns red by itself. `failed` is series-tracked, so its sparkline ticks down as the agent works. This catches the most common problem in agentic coding, an agent reporting "done" when it isn't.

Why it works: the agent isn't grading itself. Your commands compute the result from your repo, and the grid shows it next to the claim as both change.

## The document grid (`examples/review.recalctemplate`)

```
        A                          B                        C                    D                   E
 1  document                   What law governs…        Initial term…        Assign without…     Quote the passage…
 2  contracts/acme-msa.md      @claude('{instructions} Document: {$A2}. Question: {B$1}')
 3  contracts/globex-…         (fill down and right)
instructions = Answer from the document only. If the document is silent, answer "not stated". …
progress     = = count({B2:E3}) + ' of ' + … + ' answered'
```

Documents down column A (File → Import Files as Rows lists a folder for you), a question in each header, and one prompt in B2. Fill it down and right: `{$A2}` keeps its column, so every row reads its own document; `{B$1}` keeps its row, so every column asks its own question. Run Selection starts every cell; they queue under `max_agents` (config.toml, default 4) so a big table doesn't fork a big process tree. `instructions` is a named cell every prompt shares; change it once and every cell in the table goes stale, ready to re-run.

Then review: Verify (⌘Y) the answers you've checked. A verified cell is yours, so an agent's later write becomes a suggestion, and a re-fill or Run Selection skips it. Flag (⇧⌘Y) the ones to come back to, with a note. Select the block and the inspector counts answered, verified, flagged, running, and queued cells. Column E asks each agent to quote the passage its answer in D rests on, so checking an answer means reading one passage instead of searching the document.

Why it works: it covers what a "document grids" product does (rows as documents, columns as questions, a model call per cell, citations, verify and flag) using cells that read, compute, and depend on each other, with agents you already have, on files that stay on your machine.

Every answer should come with its passage: the agent calls `cite_cell` on its own cell before it answers (the task text tells it which cell it is), and the reviewer opens the document from the cell with Show Source. File → New Document Grid… and Add Question Column… build this same board without writing the formula.

## The reviewer column (`examples/reviewer.recalctemplate`)

```
        A                  B                              C                                  D
 1  document           Can either party assign…       Reviewer: is B supported? (agree / disagree)   Reviewer: if you disagree, why
 2  contracts/acme…    @claude('{instructions} …')    @codex('… Their answer: {B2}. … agree or disagree.')   @codex('… if the reviewer disagreed, say what the document supports …')
disagreements = = countif({C2:C3}, 'disagree')
```

A document grid of three hundred cells is three hundred things you didn't read. The cheapest independent check is a second agent, on its own context, told only to agree or disagree with the first one's answer after reading the document itself. Column B answers; column C is the reviewer; column D explains a disagreement in one line. `disagreements` is the number to watch, and View → Show Rows with Errors plus a filter on C is your reading list. B and C carry answer types (`@answer_type B2 B3 = yesno`, `@answer_type C2 C3 = choice:agree|disagree`), so a reviewer that writes a paragraph instead of a verdict is refused and asked again, and `countif` only ever sees the two words.

Why it works: it is omp's advisor and Akita's "do not trust a single LLM to audit anything important", expressed as a column. Using a different agent for the reviewer (Codex checking Claude, or the reverse) is one word in the prompt cell, and the disagreements between two models are the most useful signal a table gives you. The reviewer never overwrites anything; it writes its own cell, and you still verify.

## The handoff (`examples/handoff.recalctemplate`)

```
task        Summarize what this repository does in three sentences, for a new contributor.
draft       @claude('{task} Answer with the three sentences only.')
review      @codex("… It answered: {draft} Read the repository yourself … approve or revise.")
review_why  @codex("… A reviewer answered {review}. In one line, say what the answer gets wrong …")
revision    @claude('You wrote this answer to "{task}": {draft} A reviewer said {review}: {review_why} Rewrite …')
status      = if({review} == '', 'run draft, then review', if({review} == 'approve', 'approved: verify draft', 'revise: run revision'))
```

One agent drafts, a different one reviews the draft against the repo on its own, and the first revises with the review in hand. Every handoff is a cell: you can read what each agent was given and said, change a step's prompt, and verify the version you'd keep. `review` has the answer type `approve | revise`, so `status` can read it.

Why it works: it's the reviewer column for a single answer, with the revision step written down. The same handoff from inside an agent is one `ask_agent` call, which writes a cell like `review` and returns its answer, so the handoffs an agent makes land on the board too.

## Mission control (`examples/mission.recalctemplate`)

```
A2  make the tests pass              (task)
B2  @claude('do {A2}. reply done')   (the agent; its reply is its claim)
C2  = {B2_status}                    (live session status)
D2  $ cargo test 2>&1 | grep -c ok # watch: src/**
E2  = if({B2} == 'done', {D2} > 0, 'pending')
A3  write release notes …            (next row, next agent)
```

The verification board, scaled to N agents. Each row pairs an agent with its own watched ground-truth check; the agent's reply is its claim (the prompt asks for the one word `done`), so `verified` reads claim-vs-truth per row: `pending` while the agent works, green when it claims done and the check agrees, red when it claims done and the check doesn't. Session status lands in the status column; questions from every agent arrive in a single decisions queue, already attributed.

Why it works: adding an agent adds a row instead of a window. A column of `verified` cells shows who's actually green, who's still working, and who's claiming more than their tests support.

## The eval loop (`examples/eval.recalctemplate`)

```
prompt      (the thing under test — you and the agent both edit it)
run         ! python eval.py --prompt {prompt} --n {n_cases} --out results.json
score       $ jq '.mean_score' results.json
human_hint  (empty; the agent waits on this for steering)
notes       (the agent records what it changed each iteration)
```

The eval run costs money and writes files, so it's a `!` action that never fires unexpectedly; the agent triggers it explicitly each iteration. `score` only reads the results file. The agent's loop: edit `prompt` → `trigger_cell("run")` → read `score` → keep or revert, recording the change in `notes`. Meanwhile you watch `score`'s sparkline climb and drop guidance into `human_hint`; the agent has `await_cell("human_hint", "!= ''")` pending and folds your hint into the next iteration.

Why it works: the human steers an ongoing optimization with a cell edit, without interrupting it with a chat message or losing context.

## The batch board (`examples/batch.recalctemplate`)

```
A2  serde       B2  $ cargo info {A2} 2>&1 | grep -c '^error'    C2  = {B2} == 0
A3  tokio       B3  $ cargo info {A3} 2>&1 | grep -c '^error'    C3  = {B3} == 0
A4  tokio-typo  B4  $ cargo info {A4} 2>&1 | grep -c '^error'    C4  = {B4} == 0
E2  $ printf '%s\n' {C2} {C3} {C4} | grep -c true
```

One task across a column of inputs, a status expression per row, a summary at the bottom. Edit an input and only its row recomputes. An agent fills a fourth column with per-row analysis using ordinary `set_range`. That gives you batch enrichment, per-row status, selective re-run, and an audit trail (every cell keeps history), all from ordinary cells.

Why it works: it's what "AI spreadsheet" products advertise, done with no features beyond cells that read, compute, and depend on each other.

## The multi-agent lock board (`examples/multiagent.recalctemplate`)

```
A2  refactor auth module   B2 (owner)   C2 (status)
A3  migrate db layer       B3 (owner)   C3 (status)
claim_agent_a  src/auth/**     (the globs an agent will edit, written before it starts)
claim_agent_b  src/db/**
changed        $ git status --porcelain --untracked-files=all -- . ':!.recalc' | cut -c4- # every: 3000
unclaimed      $ (changed files no claim covers)
contested      $ (changed files two claims cover)
clear          = and({unclaimed} == '', {contested} == '')
```

Coordination is a convention over shared state, the Linda tuple-space pattern. An agent claims a subtask by writing its id into the owner column, claims files by writing the globs it will edit into its claim cell, and checks the other claims before editing. The checks watch what actually changed: `unclaimed` lists edits outside every claim, `contested` lists files two agents may both be editing, and `clear` goes false the moment either happens. Tried in a small repo: an edit to README.md with no claim over it showed in `unclaimed`, and widening one agent's claim over another's showed `src/auth/login.rs` in `contested`. recalc doesn't enforce any of it; it makes every lock *visible and attributed* (each write is tagged with the agent's id), and that is what keeps two agents from colliding. You triage both agents' questions from the same decisions queue.

Why it works: three chat panes are hard to watch at once, but three agents reporting locks, statuses, and questions into one grid, all attributed, are easy to supervise.

## Steering patterns worth stealing

- **The driver.** Open the Driver tab (⇧⌘D) and say what you want the board to do: "review the contracts in ./docs for assignment, termination, and governing law", "run the bench cases and explain any regression", "start three workers on the three tasks in column A and put a check beside each". It builds the table, starts the workers, and shows you what needs a look. Join its conversation from a terminal with Open in Terminal, or `recalc attach _driver`.
- **The hint cell.** Keep an empty `human_hint` cell on any board. Agents that follow `AGENTS.md` poll it between iterations, so you can type into it whenever you have something to say without interrupting them.
- **The stop cell.** `stop` starts empty; a background agent checks `await_cell("stop", "== true")`. Writing `true` stops it, and any script can do that: `recalc set stop true`, or `echo true > .recalc/cells/stop` from a process that has never heard of recalc.
- **CI as a participant.** `recalc watch deploy_ok --until "== true" --timeout 600000` blocks a shell script until the grid says go. Through the CLI, a script can read, write, and wait on the board like any agent.
- **The committed contract.** Check a board into the repo and let CI verify with it: `recalc check verify.recalc --timeout 300000` applies the board, runs its checks, and exits nonzero unless `status` reads true and is in the ok state (pick another gate with `--cell`); a gate whose formula errored is refused instead of trusted for its last value. The board you watch locally is the gate in CI, and a failure prints the full board state so the log shows *why*.
