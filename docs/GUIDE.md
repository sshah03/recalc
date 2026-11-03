# The recalc guide

Everything recalc does, in more depth than the [README](../README.md). Start there if you're new; come here for the details. recalc is in early beta, so some of this will change.

- [What it's for](#what-its-for)
- [Install and connect an agent](#install-and-connect-an-agent)
- [Cells](#cells)
- [Document grids](#document-grids)
- [Agents in cells](#agents-in-cells)
- [The Driver, and agents working together](#the-driver-and-agents-working-together)
- [Reviewing](#reviewing)
- [Holding an agent's "done" to the board](#holding-an-agents-done-to-the-board)
- [Scripts, the command line, and CI](#scripts-the-command-line-and-ci)
- [Boards and files](#boards-and-files)
- [Settings](#settings)
- [Examples and templates](#examples-and-templates)
- [Why a spreadsheet](#why-a-spreadsheet)
- [What recalc is not](#what-recalc-is-not)

## What it's for

**Checking an agent's work.** An agent works for twenty minutes and reports "done, all tests passing". You can take its word for it or scroll back through the transcript, and once you've checked, the transcript scrolls away. In recalc you keep a board like this open beside the agent:

```
          A               B                 C
 1  Test Suite      3 passed
 2  raw             1 failed  ▁▂▄▂▁          claim_done   true    a1
 3  status          false                    verified     false
```

`raw` runs your test suite and watches your repo (`$ cargo test 2>&1 # watch: src/**`), so every edit, yours or the agent's, re-runs it. `status` reads the suite's exit code (`= {raw.code} == 0`). `claim_done` is where the agent writes `true` when it believes it's finished, and `verified` is `= claim_done == status`. If the agent claims done while a test is red, `verified` turns red next to the claim. The agent isn't reporting on itself: the board runs *your* tests against *your* repo and shows the result beside its claim.

**Asking many documents the same questions.** Documents down column A, a question in each column header, and one agent prompt filled across the block. Every answer lands in its cell with the passage it came from, and you sign off on the ones you've checked. See [Document grids](#document-grids).

## Install and connect an agent

Install the app and the `recalc` command:

```
curl -fsSL https://raw.githubusercontent.com/sshah03/recalc/main/install.sh | sh
```

or download a build from Releases (the README says how to open an unsigned beta), or build it yourself:

```
cd desktop/frontend && npm install && npm run build
cd ../.. && cargo build --release -p recalc-desktop     # → target/release/recalc
```

Then, in a project, `recalc` opens the board in that folder. Opened from Finder or the Dock, the app reopens your last board, or starts an Untitled board in `~/Documents/recalc boards`. Without the install script, recalc → Install Command Line Tool… (Help → Install Command Line Tool… on Linux) links `recalc` into `~/.local/bin` when your PATH has it, otherwise `/usr/local/bin`, which asks for your password.

recalc runs the agents you already use, Claude Code and Codex, signed in to your own plan. Launched from the Dock, it adopts your login shell's `PATH`, so `claude`, `codex`, and whatever your cells call resolve as they do in a terminal. Agent → Connect an Agent… shows where each one resolves; if one is somewhere unusual, name it under `[paths]` in [settings](#settings).

To let an agent you run yourself work on the board, add recalc as an MCP server. For Claude Code:

```
claude mcp add recalc -- recalc mcp
```

Connect an Agent… shows this line with the app's full path, ready to copy. When the agent connects it's told what the board is, and gets tools like `get_cell`, `set_cell` and `await_cell` ([docs/tools](tools/) has one page per tool). After that, work as usual: the grid fills in as the agent reads and writes cells, and you edit cells to steer. Copy `AGENTS.md` (and the thin `CLAUDE.md`) from this repo into yours so agents pick up the working habits.

## Cells

The first character of a cell says what kind it is:

```
42                          a value
= failed == 0               a formula over other cells
$ cargo test 2>&1           a command; its output is the value
! git commit -am {msg}      an action with side effects
@claude('prompt', {refs})   an agent call
```

The rule to remember: **`$` commands recompute on their own; `!` actions and `@` agents never do.** A cell that reads (`grep`, `wc`, a test run) is `$` and refreshes when its inputs change. A cell that changes something (a commit, a deploy, an agent) is `!` or `@` and runs only when you run it: Run (⌘↩), Run Selection (⇧⌘↩), or Run Column (⌥⌘↩). Running a selection skips cells you've verified and cells already running. Stop (⌘.) ends a run and keeps the cell's last value; Stop All Runs is ⇧⌘.

### References

`{name}` pulls in another cell's value and makes that cell an input: when it changes, the commands and formulas that read it re-run. Cells have grid addresses (`{B2}`) or names (Edit → New Named Cell…, ⌥⌘N). A1 references take `$` anchors (`{$A$1}`, `{A$1}`, `{$A1}`), which fill respects and F4 cycles while you edit. A board can have more than one sheet: `{Notes!A2}` (or `{'My Notes'!A2}`) reads a cell on another one, and a plain `{A2}` means the cell's own sheet.

In a command, the value is shell-quoted, so it can't inject shell syntax, whether the reference sits bare (`grep {pattern} file`) or inside quotes (`echo "found {pattern}"`). `{name:raw}` opts out of quoting, and `{name:file}` passes the path of a file holding the value, for big outputs. A command's exit code is `{name.code}`; an agent cell's model is `{name.model}`.

### Formulas

Formulas are kept small:

- references, and arithmetic with `+ - * /`;
- `&` to join text;
- comparisons (`==`, `!=`, `<`, `>`, `<=`, `>=`);
- a short function set that Sheets users will recognize:
  - aggregates: `sum`, `avg`, `count`, `min`, `max`, `median`, `countif`, `countifs`, `sumif`;
  - numbers: `round`, `abs`;
  - logic: `if`, `and`, `or`, `not`;
  - text: `concat`, `len`, `lower`, `upper`, `trim`;
  - lookup: `lookup`, `match`.

Arithmetic works as it does in Excel:

- `+` adds numbers only; join text with `&`.
- A blank cell counts as 0.
- Numbers are read as people write them: `$1,200`, `1,000.50`, and `5%` (0.05). A `$` followed by a number is an amount; a command is `$`, a space, and the command.

One difference from Excel: a formula over an agent's answer that hasn't come in yet waits for it instead of counting it as 0. Until the answer arrives, `==`, text functions, and counts read the cell as blank, so a "how many are answered" count works while runs are going. If the run failed, the formula shows its error. For anything beyond this function set, use a command.

### Commands that watch

A `$` cell re-runs when a cell it reads changes. It can also re-run on other triggers:

- **Files:** `{file:src/main.rs}` and `{glob:src/**/*.rs}` make files inputs.
- **Watch annotation:** a trailing `# watch: <glob>` re-runs the cell when matching files change. Separate several globs with spaces (`# watch: src/** tests/*.py`). `# watch: .git/HEAD` re-runs it when you switch branches or commit.
- **Timer:** `# every: <ms>` re-runs it on a period.
- **History:** Cell → Show / Hide History Chart draws a cell's past numbers as a sparkline on its face.

### Rows that stay in sync

A `$` cell's output can become rows:

```
$ gh pr list --json number,title # rows: A2 key=number cols=number,title gone=E
```

This writes one row per PR from A2 down, matched by `number`:

- **Each run** updates the fields of rows already there. It leaves the other columns alone: agent answers, checks, your marks.
- **A new PR** gets a new row, with the formulas, commands, and agents of the row above filled in.
- **A PR that's gone** keeps its row, grey, with `gone` in column E.

The output can be:

- **a JSON array of objects;**
- **a JSON object of objects:** the property name becomes the field `_key`, which is the shape `npm outdated --json` gives;
- **plain lines:** without `key=`, each line of output is a row.

`author.login` reads a nested field. Add `# every: 60000` to re-sync every minute.

### Editing like a spreadsheet

Rows and columns insert, delete, and sort like any spreadsheet's. References follow their cells, so a `sum` over a block still covers the block after a sort. A reference to a deleted cell becomes `#REF!` instead of reading whatever moved into its place.

- **Undo** (⌘Z) takes back a delete, an insert, or a sort, and a whole paste or fill in one step.
- **Fill** with Fill Down (⌘D) and Fill Right (⌘R).
- **Paste Values** (⇧⌘V) pastes plain values.
- **Formats:** cells take bold, italic, alignment, a fill color, and a number format (currency, percent, date, and so on). A number format changes how a value is shown, never what the file holds.
- **Viewing:** Make Header Row keeps row 1 in view and out of sorts. Freeze Panes at Selection pins rows and columns, Wrap Text (⌥⌘W) shows long values in full, and hovering a cell shows its whole value too.
- **Finding:** Find (⌘F) searches across cells. The Command Palette (⌘K) finds any menu item, and the Cheatsheet (⌘/) lists the shortcuts.

## Document grids

To start one, choose File → New Board… → Document grid from a folder. Each document in the folder gets a row (PDFs and Word files are read as text). Then Add Question Column… (⇧⌘Q) takes:

- **the question;**
- **its answer type:** a sentence, yes/no, a date, a number, an amount, one of a fixed set, a verbatim quote, or JSON;
- **the column it asks about:** usually the documents, but a column of company names makes each name the row's subject;
- **optionally, the agent and model** its cells run on.

It writes the header, the prompts, and the number format for you, from a library of columns you can add to. Run Column (⌥⌘↩) or Run Selection then starts the agents. Runs queue under `max_agents` (four by default), so fifty prompts don't mean fifty processes.

The same grid by hand is one formula, filled down and right:

```
B2 = @claude('{instructions} Document: {$A2}. Question: {B$1}')
```

`{B$1}` keeps its row (the question) and `{$A2}` keeps its column (the document), so every cell gets its own pair. File → Import Files as Rows… puts a folder's documents down column A.

**Citations.** Every answer carries the file and passage the agent relied on. recalc checks that the passage is really in the file. If it isn't, the cell is marked, the problem is listed in Needs You, and the agent is told so it can quote the document's exact words. A cited cell shows a small quote mark, and the Inspector shows the passage. Show Source (⌥⌘S) opens the document beside the grid with the passage highlighted.

**Answer types.** A typed column holds its answers to their type:

- **Wrong-shape answers are sent back.** An agent that writes prose where a yes/no, date, or number belongs is told what was wanted, and answers again.
- **Misfit output never reaches formulas.** A run that prints a misfit lands red, with the answer in its console, so formulas never see a malformed value.
- **JSON** takes one object or array, optionally with keys it must have. Prose or a code fence around the JSON is dropped. An object missing a key is sent back, naming the key.
- **Typing yourself** in a typed cell offers what it takes: its options, or Yes and No (▾ on the cell lists them), or a calendar. A date you type ("oct 3") becomes YYYY-MM-DD. A value of yours that doesn't fit is kept, since you may be correcting the type, and the Inspector says what the cell takes.

**Documents outside the board.** A grid over a folder picked from anywhere works. A cell's agent is allowed to read the folder holding the document it names, and only that folder.

**Where the documents go.** The files stay on your machine, but the agent reads them. With Claude Code or Codex, the text of each document it answers from goes to Anthropic or OpenAI, like any other prompt. For confidential documents, run the grid on a local model and nothing leaves the machine: [LOCAL.md](LOCAL.md) has the two-line setup.

Help → Document Grids Guide walks through the flow in the app, with notes for contract review, document intake, and research reading.

## Agents in cells

Type `@claude('fix the flaky auth test')` (or `@codex(…)`; New Agent Cell, ⇧⌘A, starts one). Then either:

- **Run** (⌘↩) for a headless run whose answer becomes the cell's value. The cell shows what the agent is doing while it works.
- **Open Session** (⇧⌘T) for the agent's interactive terminal, live beside the grid. It writes its answer into the cell when it has one.

Click an agent cell and the Inspector shows the prompt it ran, the inputs it read, the output, the model, and what the run cost.

**Picking a model.** Add it as the last argument: `@claude('summarize {A2}', model='haiku')`. The name goes to the agent as written, or is looked up under `[models]` in [settings](#settings) (`fast = "claude-haiku-5-5"`). `{B2.model}` reads which model answered, so a table can compare models cell by cell.

**Conversations you can go back to.** Every agent cell keeps its conversation id.

- **Joining from a terminal.** When tmux is installed, a session runs inside it, so it outlives the window. Open in Terminal (⌥⌘T) joins the same conversation in a terminal window, and so does `recalc attach B2` from a shell. Terminal runs it through a "recalc" profile, added the first time, so nothing in your shell's startup gets in the way. The same command resumes a finished headless run, which is the quickest way to ask an agent *why* it answered as it did.

  https://github.com/user-attachments/assets/9f2c745a-c177-41c1-870e-e6aaaca8b2a7

- **After a restart,** when tmux and its sessions are gone, the board still has every id. Resume session in the Inspector continues the cell's last conversation, and More → New Conversation starts over.
- **On another computer,** a cell's session starts a new conversation and says so. It begins with a recap of what the board kept: the answer and who wrote it, your marks and notes, its citation, and its earlier answers.

**Permissions and limits.**

- **Tool requests.** A headless Claude run that wants a tool it wasn't given (Bash, say) asks you in Needs You, showing the exact command, and waits. The wait doesn't count against its timeout. Codex runs don't ask, because `codex exec` has no way to.
- **Usage limits.** If an agent's account hits its usage limit, queued agent cells wait until it lifts instead of all failing. The status bar says until when, and running one yourself tries sooner.
- **Commands.** An agent computing a cell can't write or run `$` or `!` cells.

**Costs.** Every run records its tokens, turns, and cost at API prices. The Inspector shows the cost for a cell, summed over a selection, and the status bar shows it for the whole board, the Driver's sessions included. On a Claude or ChatGPT plan nothing is billed per run; the dollars say how much of the plan's usage the runs took. Agent → Set Agent Budget… caps a board: past it, queued agent cells wait for you, and a cell you run yourself still starts.

**Changing inputs.** An agent that has read cells is working from a snapshot of them. When one of those cells changes (you edited it, another agent wrote it, a check re-ran), what happens depends on how the agent is working:

- **An agent connected over MCP** gets a `changed_inputs` block in its next tool result: the cell, the value it saw, and the value now. Cells it never read don't appear.
- **An agent in a live session** may not be calling tools at all. Its cell shows `inputs changed`, and the Inspector lists the diff. Agent → Tell Agent About Changed Inputs types the diff into its terminal, as you would by hand. recalc never types into an agent's terminal on its own, because text arriving while the agent is asking a question could answer it.

**Other agents, or your own flags.** Name them under `[agents]` in [settings](#settings). In a template, `{task}` is the prompt and `{model}` is the `model=` argument, and `@name('…')` runs it.

- **Keeping an agent to one folder.** Start the command with `cd folder &&`, since Claude Code in print mode can't read outside the folder it starts in: `contestant = "cd arena && claude -p {task} {model} --output-format json --strict-mcp-config --allowedTools 'Read(./**)'"` reads `arena/` and nothing else on the board.
- **Showing progress and asking.** Write a template the way the builtin one is written, with the prompt on stdin: `claude -p {model} --input-format stream-json --output-format stream-json --verbose --permission-prompt-tool stdio …` with no `{task}`, or `codex exec {model} --json … -`. Its runs then show what they're doing, and a tool it isn't allowed asks you in Needs You. Leave out `--permission-prompt-tool stdio` when a run should be refused instead of asking, as an eval's contestants should be.
- **Resuming.** A template that runs `claude` or `codex exec` resumes like the builtin one, from the same folder and on the cell's model. For any other program, say how under `[resume]`: `mine = "mine --resume {session}"`.

## The Driver, and agents working together

The **Driver** (⇧⌘D; ⌥⌘D widens it) seats a Claude Code or Codex session in the side panel that is told what the grid is and how to use it. Ask it to set up a document grid over a folder, pull a dataset off the web into a column, or start three workers on three tasks with a check beside each. It fills cells, starts other agents, and points you at what needs a look. It cannot verify; that stays with you.

- **Starting from it.** File → New Board… → Describe it to the Driver starts a board this way.
- **Answering from one conversation.** Cell → Ask the Driver to Answer has the Driver answer the selected agent cells itself, in its own conversation, instead of running each as an agent of its own.
- **Choosing who sits there.** While the seat is empty, pick Claude Code, Codex, or one of your `[agents]` templates from the Driver tab. The choice is saved as `driver` in settings.
- **Resuming.** Resume continues its conversation. Start new begins fresh, picking up from the plan and facts it kept in cells.

Agents work with each other the way they work with you: through cells. There's no channel between agents that isn't a cell, and agents started by agents stop three deep.

- **Handing off a question.** `ask_agent` hands a question to another agent (Codex reviewing Claude's draft, say). It writes the question into a cell as an `@codex(…)` call, runs it, and returns the answer, so the handoff sits on the board where you can read and check it.
- **Naming who may answer.** A multiple-choice question an agent puts to the board (`ask_cell`) can name another agent with `for`. You can always answer it yourself.

## Reviewing

**Needs You** (⌘J) is one list of everything waiting on you: questions agents asked, suggestions on cells they own, sessions whose inputs moved, errors, flags, and answers that finished since you last looked, each with how long it has waited. Below them is what's running, with a timer and what each agent is doing.

- **New rows go at the bottom,** so nothing moves under your pointer.
- **Answering a question:** press its number. The agent's recommended option is marked, but never chosen for you.
- **Notifications:** when recalc is in the background, a desktop notification says what arrived, and coming back selects that cell. View → Notify When Something Needs You turns these off.

**Verify** a cell (⌘Y) and it's yours. An agent that tries to overwrite it has its write held as a suggestion for you instead, and a re-fill or a Run Selection skips it.

**Flag** a cell (⇧⌘Y) with a note to come back to. Run a flagged agent cell again and its agent is told what you flagged and why. It takes your note as guidance on what the question means and looks again.

To go through many answers:

- **Review Answers** (⌥⌘R) steps through unverified answers column by column: V verifies, F flags, J skips, K goes back, and Esc stops.
- **Next Unverified Answer** (⌥⌘↓) jumps to the next one, and Verify and Next (⌥⌘Y) signs off and moves on.
- **The Inspector** counts answered, verified, flagged, running, and queued cells in a selection.
- **The View menu** filters the sheet to flagged, unverified, errored, or running rows. Filter to This Value and Filter Column… narrow it to rows where a column holds a value you pick, and filters on several columns combine.

**Editing an agent's cell.** recalc doesn't silently overwrite the agent's work. It asks what you meant: suggest the change to the agent, pin your value, or override once.

**When you're done,** File → Export CSV… or Export Excel Workbook… writes the table. The workbook keeps formats, with each citation and mark as a cell note. Edit → Copy as Markdown Table (⇧⌘C) copies the selection for a memo.

## Holding an agent's "done" to the board

A Claude Code session can be kept from finishing until its row on the board passes. Add `recalc hook stop` as its `Stop` hook in `.claude/settings.json`:

```json
{ "hooks": { "Stop": [{ "hooks": [{ "type": "command", "timeout": 90,
  "command": "recalc hook stop --task A --session C --claim D --gate F || true" }] }] } }
```

When the session tries to finish, the hook:

1. **Finds the session's row:**
   - the agent cell the session started from, if a cell started it;
   - otherwise, its session id in column C;
   - otherwise, a task name in column A that is part of the session's folder, as in a worktree `wt/parse`.
2. **Writes** the session id into C and what the session said into D.
3. **Waits** for the row's checks to settle.
4. **Reads F,** the gate. Unless F reads `holds`, `pass`, `passes`, `true`, `yes`, or `ok` (set others with `--pass`), the session isn't allowed to stop. It gets the failing checks' last lines and keeps working.

After three blocks (`--max-blocks`), the session may stop and F is flagged for you. A cell's headless runs aren't held, since their answer is the cell's value.

Keep the `|| true`. Claude Code reads a Stop hook's exit status 2 as "block", so a `recalc` too old to know `hook` would otherwise keep the session going.

Codex reads the same hook from `.codex/hooks.json` (the same JSON, under `hooks.Stop`), once you trust it: open `codex` in the folder and accept its hook review. Until then Codex skips the hook without saying so, `codex exec` included. The "Done means green" example ships both files.

## Scripts, the command line, and CI

A board's scripts are its `!` cells. For example, `normalize_dates = ! python3 scripts/normalize_dates.py {B2:B50}` runs a script in the board's folder, with each value of B2:B50 as an argument. The script can:

- read more with `recalc get 'B2:B50'` (a single range prints just its rows);
- write a block back by piping CSV to `recalc import - --at C2`.

You run an `!` cell yourself, and the first run of one an agent wrote is always yours. ⌘K lists the board's scripts beside its commands, and the Inspector's Edit Script opens the file in your text editor. Agents are told to save a change someone could want again this way (cleaning a column, reformatting dates), rather than as hundreds of one-off writes.

From any shell, in the board's folder:

| | |
|---|---|
| `recalc get B2 'Rules!A1:B6'` | print values |
| `recalc set B2 '= {A2} * 2'` | set a cell |
| `recalc trigger deploy` | run an `!` cell, or refresh a `$` cell |
| `recalc watch status --until "== true"` | wait until a cell changes, or matches |
| `recalc resolve B2 1` | answer an agent's question |
| `recalc list --json` | every cell |
| `recalc import data.csv --at A2` | CSV into cells (`-` reads stdin) |
| `recalc attach B2` | continue a cell's agent conversation here |
| `recalc run --task "…"` | drive an agent headless with the grid as its only interface |
| `recalc demo [name]` | list the example boards, or unpack one and open it |

`recalc check verify.recalctemplate --timeout 300000` applies a board, runs its checks, and exits nonzero unless the gate cell (`--cell`, `status` by default) reads true *and* is in the ok state. A gate whose formula errored keeps its last value for people to read, but the check refuses it. The board you watch locally is the gate in CI. A failure prints the full board state, so the log shows *why*.

## Boards and files

A board is its folder.

- **A board in a repo** lives in the repo's `.recalc/`, committed or not as you like (File → Open Folder…).
- **A board of its own** (File → New Board…) is a `Name.recalc` folder that macOS shows as one document; double-click it to open it. New Board opens an Untitled board in `~/Documents/recalc boards`. Clicking the board's name at the top of the window, or File → Rename… and Move To…, renames or moves it. A new board you leave untouched is removed.

recalc saves every change as you make it. File → Save a Copy… (⇧⌘S) copies the board to a new name and place, and carries on in the copy with fresh agent conversations.

**Sharing.**

- **To send a copy,** use File → Export Board… (or `recalc export Leases`). It writes `Leases.recalc.zip`: the folder as git would see it, plus copies of any documents its cells name from elsewhere. Unzip it, or pick the zip in File → Open…; the first time the board opens, those cells point at the copies. An export is a copy, like an emailed spreadsheet, so changes to it don't come back.
- **To work on one board together,** share the folder itself: through git for a board in a repo, or a shared drive for a board of its own.
- **Templates.** File → Save Board as Template… (`recalc save`) writes the cells as text, `Name.recalctemplate`. Opening one starts a new board from it.

**Sheets.** Tabs under the grid hold the board's sheets. Right-click a tab → Duplicate Sheet… copies a sheet as a fresh one. The copy has the formulas, commands, and agent prompts as written, with formats and answer types, but not the agents' answers or your marks. One client's workpaper becomes the next client's by changing its inputs.

**Plain files underneath.** State lives in plain files under `.recalc/`, one per cell: `cat .recalc/cells/status` prints a value, and each extra sheet is a folder under `.recalc/sheets/`. Any script, any `echo`, or any second agent that has never heard of recalc can take part by reading and writing these files. The app and a headless agent share state the same way, with no separate channel. `.recalc/.gitignore` keeps out what belongs to one computer: what's running, and agent settings that name its paths.

## Settings

A board's settings are in `.recalc/config.toml`. Every key is optional:

| Key | What it does |
|---|---|
| `max_agents` | how many agent runs at once; the rest queue (default 4) |
| `agent_timeout_ms` | how long an agent run may take (default 600000, ten minutes) |
| `timeout_ms` | how long a `$` or `!` command may take (default 30000) |
| `shell`, `cwd`, `env` | how commands run: the shell (default `sh`), the folder, extra environment variables |
| `budget_usd` | the board's agent spend, at API prices, after which queued agent cells wait (Set Agent Budget… writes it) |
| `driver` | who sits in the Driver seat: `claude`, `codex`, or an `[agents]` name |
| `danger_tokens` | extra words that quarantine a `$` cell, on top of the built-in list |
| `[paths]` | where an agent lives when it isn't on your PATH: `claude = "/opt/tools/claude"` |
| `[models]` | short names for `model=`: `fast = "claude-haiku-5-5"` |
| `[agents]` | agent command templates, run as `@name(…)` |
| `[resume]` | how to continue an `[agents]` template's conversation, for programs other than `claude` and `codex` |

To change what agents are told on one board, write `.recalc/prompts/instructions.local.md` (what every agent gets when it connects) or `.recalc/prompts/driver.local.md` (the Driver's role). recalc's defaults are in [src/prompts/](../src/prompts/).

**Safety.** Side effects are *declared*, and never run automatically:

- **An agent can write an `!` action but can't run it first.** It waits for a person to run it once.
- **Command cells are scanned.** recalc quarantines a `$` cell if it spots destructive words in it (`danger_tokens` adds to the list). This guards against an agent's mistake; it isn't a security boundary.

## Examples and templates

File → New Board… lists the examples. Each opens in a new folder with its sample files or a sample repo. `recalc demo <name>` does the same from a shell.

| Name | What it shows |
|---|---|
| `legal` | six contracts read side by side: typed columns, an amended agreement read as one row, risk columns computed from other columns, a citation behind each answer |
| `tax` | five tax forms read into numbers and reconciled against the client's own summary, where one line comes back red |
| `practice` | one question across eight clients of a tax practice, a missing-documents check that re-runs when a folder changes, and request letters drafted as actions |
| `entities` | K-1s from an S corporation, a partnership, and a trust, checked against the client's 1040 |
| `scorp` | five S-corp clients' books classified onto the IRS's draft 2026 Form 1120-S, one sheet per client, checks that tie each return out, and an action that fills the PDF |
| `research` | a literature matrix over five abstracts, and a synthesis cell fed the full table |
| `done-means-green` | Claude Code sessions that can't finish until their tests pass |
| `flaky-hunt` | suspect tests run 50 times, each cause named, and every fix proved |
| `review-lanes` | AI code reviewers scored on open pull requests by how often they're right |
| `release-gate` | checks, a sign-off, and release notes that must all pass before you tag |
| `beads-board` | beads issues as rows, each held to its test before it closes |
| `claims-ledger` | four agents in one repo, each claim rechecked as the code changes |
| `cve-triage` | scanner advisories checked against the code you actually ship |
| `bench` | benchmark scores tracked over time, with a regression gate and an agent to explain drift |
| `models` | Claude and Codex given the same tasks, graded, with an agent judge for free text |
| `sec-questionnaire` | a customer's security questions answered from your policies, checked against your Terraform |
| `soc2-evidence` | ten controls tested over six months of exports, with checks an auditor can re-run |

[demos/README.md](../demos/README.md) walks through each one: what to try, and what should happen.

File → Insert Template… adds a pattern's cells to the board you have open:

- the verification board;
- the document grid, and a reviewer column;
- mission control for several agents;
- a batch board;
- an eval loop;
- multi-agent locks;
- draft, review, revise.

[RECIPES.md](../RECIPES.md) explains why each one works.

## Why a spreadsheet

A coding agent's working state lives in three awkward places:

- **the context window,** which vanishes when it's compacted;
- **the filesystem,** which is persistent but unstructured, and you can't watch it change;
- **the transcript,** which is linear, and scrolls away.

None of them lets you and the agent both see and edit the same structured state as it changes.

The idea is an old one: a *tuple space*, where independent processes coordinate by reading and writing shared, addressable memory instead of messaging each other. A spreadsheet is a tuple space with forty years of UI work behind it: addressable cells, reactive dependencies, and a grid that's easy to scan. recalc is a spreadsheet whose cells can also run shell commands and agents.

## What recalc is not

- **Not an AI spreadsheet.** Quadratic, Copilot in Excel, and similar tools put a model *inside* a spreadsheet to write formulas for you. recalc is a coordination surface *between* you and agents you already run, local and file-backed, with no model inside it. It doesn't do the work; it makes the work visible and checkable.
- **Not Excel.** The function set is finite and every addition needs a reason; shell is the formula language.
- **Not a cloud service.** There are no accounts, no server, and no telemetry.
  - The one request recalc makes on its own is a daily check for a new version against its GitHub releases (Check for Updates… turns it off). Updates install only when signed with the project's key.
  - The agents you run send what they send to their own providers.
  - Sharing a board means sharing a folder.
- **Not a sandbox.** It shows and coordinates what commands do; it does not contain them. Run it where you'd run the commands anyway.
- **Not automatic verification.** It runs the checks you write and puts the results next to the agent's claims. Verified means *you* looked: recalc records that you did, and stops agents from overwriting it.
- **Not a launcher.** T3 Code, Conductor, and Claude Code's own Agent View start agents, give each a worktree, and turn the result into a PR; use them for that. Keep a recalc board open beside them for what they don't do: the claim and the check in the same row, a table over many documents, and a sign-off an agent can't overwrite. The Driver does start agents, but by writing cells the board runs, and you can move any cell's session to a terminal at any moment.

## If it crashes

A crash writes the panic and a backtrace to `~/Library/Logs/recalc/crash.log` on macOS (`~/.local/state/recalc/crash.log` on Linux) before the app exits. Help → Report a Problem… opens a new issue; please attach the log to it.

## Repository

```
src/                 the core: engine, store, expression language, MCP server, CLI, tmux sessions
src/prompts/         what agents are told: the cell footer, review instructions, the MCP instructions, the Driver's role
desktop/src-tauri/   the app: native menus, the board behind the window, terminal cells
desktop/frontend/    the UI: TypeScript, vanilla DOM, no bundler
examples/            the templates (File → Insert Template…)
demos/               the example boards (`recalc demo <name>`), each with its sample data
AGENTS.md            working habits for agents on a recalc board; copy it into your repo
RECIPES.md           the boards people actually keep, and why they work
docs/tools/          one page per MCP tool: inputs, outputs, flow, errors
docs/RELEASING.md    cutting a release, signing secrets, re-recording agent fixtures
tests/fixtures/      what Claude Code and Codex really print, per CLI version, for the parser tests
CONTRIBUTING.md      the checks, and the hit-every-surface list
```

Development notes are in [desktop/README.md](../desktop/README.md).
