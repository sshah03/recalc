# Demos

Each demo is a folder with a board and the sample data it reads, so it runs on a
fresh machine. Open one from File → New Board…, or unpack it into a new
directory from a shell:

```
recalc demo legal          # creates ./recalc-legal, applies the board, opens the window
recalc demo                # lists them
recalc demo tax --into ~/scratch/tax-demo
```

The agent cells need `claude` (Claude Code) and, for the models and entities
demos, `codex`. Agent → Connect an Agent shows whether they resolve; if one is
installed somewhere unusual, set `claude = "/path/to/claude"` under `[paths]`
in the board's `.recalc/config.toml`. Nothing runs until you tell it to: select
the block of agent cells and press Run Selection (⇧⌘↩). Runs queue under
`max_agents` (four by default). A headless run may read files in the board
directory and write cells; it can't change anything else. Every company,
person, document and repository is synthetic. A single cell takes about
fifteen seconds; a full table, a few minutes.

The answer keys stay in this repository; `recalc demo` doesn't unpack them, so
the agents can't read them.

**Documents**

- [legal](#legal-due-diligence-document-grid): six contracts read side by side
- [tax](#tax-document-intake-and-reconciliation): five tax forms reconciled against the client's summary
- [practice](#practice-one-question-across-every-client): one question across eight clients of a tax practice
- [entities](#entities-one-clients-linked-returns): K-1s checked against the client's 1040
- [scorp](#scorp-s-corp-returns-on-the-irss-draft-form): five S-corp returns on the IRS's draft Form 1120-S, one sheet per client
- [research](#research-a-literature-matrix-with-a-synthesis-cell): a literature matrix and a synthesis cell

**Coding agents and checks**

- [done-means-green](#done-means-green-sessions-that-cant-finish-red): Claude Code sessions that can't finish until their tests pass
- [claims-ledger](#claims-ledger-four-agents-one-repository): four agents in one repo, each "done" rechecked as the code changes
- [flaky-hunt](#flaky-hunt-prove-the-fix): suspect tests run 50 times, each cause named, every fix proved
- [beads-board](#beads-board-beads-decides-whats-next-the-board-decides-whats-done): beads issues held to their tests before they close
- [review-lanes](#review-lanes-which-ai-reviewer-is-worth-keeping): AI code review lanes scored by how often they're right
- [release-gate](#release-gate-checks-a-sign-off-and-a-tag-that-waits): a tag that refuses until every check and sign-off passes
- [cve-triage](#cve-triage-advisories-against-the-code-that-ships): scanner advisories checked against the code you ship
- [bench](#bench-benchmarks-that-keep-score): benchmark scores tracked over time, with a regression gate
- [models](#models-the-same-tasks-two-agents-a-score): Claude and Codex on the same tasks, graded

**Security and compliance**

- [sec-questionnaire](#sec-questionnaire-answers-checked-against-the-terraform): a customer's security questions answered from your policies and checked against your infrastructure
- [soc2-evidence](#soc2-evidence-controls-re-performed-by-commands): ten SOC 2 controls tested over six months of exports

## legal: due-diligence document grid

The document grid legal teams run in diligence: the target company's
contracts down column A, one field per header, an agent answer in every cell,
and the type of each answer named in its header (a date, a classification
with fixed choices, a currency phrase, a verbatim quote).

Three things to notice:

- **Row 2 is a file group.** The Acme MSA and its Amendment No. 1 are read
  together, and the instructions say the amendment controls. The original
  caps liability at 12 months' fees and lets either side assign to a
  successor; the amendment raises the cap to 2x and adds a change-of-control
  consent right. The answers should reflect the amendment.
- **Every answer is cited.** The agent attaches the passage it relied on;
  the cell shows a small quote mark, the inspector shows the passage, and
  Show Source opens the contract beside the grid with it highlighted, so
  checking an answer means reading one passage.
- **Columns J and K are conditional.** They don't read the documents; they
  rate change-of-control risk from the answers in F, G and I, and re-run when
  those change.

`consent_required`, `uncapped`, and `high_risk` count what the deal team
cares about. Verify (⌘Y) each answer you've checked; flag (⇧⌘Y) with a note
in the vocabulary the team uses ("deal-breaker", "negotiate", "standard").
The block summary in the inspector counts answered, verified, and flagged
cells. When the table is full, run `deal_summary`: it's given the full table as context
and writes the change-of-control section of the memo from it.

What to expect: the lease treats a change of control as an assignment needing
landlord consent (with a net-worth carve-out); the employment agreement has a
twelve-month non-compete and double severance on a post-acquisition
termination; the NDA is silent on liability caps.

## tax: document intake and reconciliation

Five forms (a W-2, four 1099s) down column A, one field per header. The
headers tell the agent the answer type (a short name, a form type, a
plain number), so formulas can use the answers directly: `wages`, `interest`,
`consulting`, `dividends`, `stock_gain` look each amount up by form type;
`total_income` and `se_tax_estimate` follow; `wages_match` and `gain_match`
compare against what the client said at intake. One of them comes back red:
the 1099-B says 3,500 and the client remembered 3,200. Flag the row, fix
`client_gain`, and watch it turn green.

This demonstrates the board's structure and is not tax advice.

## practice: one question across every client

The client grid a tax practice runs at the start of a season: eight client
folders down column A (five individuals, an S corporation, a partnership, a
trust), each holding a `profile.md` and the documents received so far. Agents
read return type, status, and assignee from the profile, then prior-year AGI
and total tax, QBI eligibility, and estimated payments from the documents,
each answer cited. Formulas turn those into an effective tax rate, a
behind-on-estimates flag, and the counts across the top: ready to file,
waiting on the client, the median rate, clients per preparer.

A product built for this has a client grid too. What a general tool adds is
that each column can be a different kind of cell. Column H isn't an agent:
it's a `$` check (`tools/missing_docs.py`) that compares the documents each
profile expects with the files actually in the folder. It re-runs whenever
anything under `clients/` changes, and it gives the same answer every time.
It finds three gaps before any agent runs. Column I compares it with the
status, and Brennan comes back red: marked Ready to file, but the Ridgeline
K-1 from last year's return never came. Column N drafts the request letter
(`letters/<client>.md`) as a `!` action. A person runs the first one; the
others can follow as a batch.

Hard cases: a CORRECTED 1099-DIV that adds Section 199A dividends (QBI is Yes
only from the corrected form); a 1041 whose counterpart to AGI is "adjusted
total income", next to two other income lines; a Direct Pay export that
starts with last year's fourth-quarter payment (counting it hides an unpaid
quarter); and two pass-throughs whose rate reads n/a, so the median skips
them. The answer key is `demos/practice/expected.json` in this repository.

## entities: one client's linked returns

The entity grid for one family: Rosalind Kettering's S corporation,
partnership, and family trust, one row each, every row a file group (the
entity's return summary and the K-1s it issued), plus her own draft 1040 on
the last row. Agents read the return type, her share, the entity's income, and
the amount on her K-1. Column F checks that the K-1 equals the income times
her share. Column G is a `$` check (`tools/schedule_e.py`) that finds the
line on her 1040 with that entity's EIN and reads what it reports, and
column H compares the two. Column I is a second model: Codex reads the same
files and agrees or disagrees with E.

`mismatches` should come back 1. The S corporation's K-1 says 87,450.00 and
the 1040 carries 78,450.00, a transposition, so `gap` reads 9000. The
partnership row holds an original K-1 at 20% and an Amended K-1 at 25%. The
amended one controls, and an answer taken from the original turns H3 red. Fix
the 1040 summary and G re-runs by itself. The answer key is
`demos/entities/expected.json` in this repository.

Both boards show the board's structure and are not tax advice.

## scorp: S-corp returns on the IRS's draft form

A small firm's S-corp season: five fictional clients' 2026 books, carried onto
the IRS's draft 2026 Form 1120-S. The board has three sheets to start:

- **Clients:** one row per client, with each return's status, ordinary income,
  distributions and owner pay.
- **Form map:** the draft's lines and field names, read from the IRS's own PDF
  in `irs/`.
- **Harbor:** the firm's workpaper for the first client.

Run **Setup** on the Clients sheet once (H2). It installs `pypdf` into `.venv`
in the board's folder, and the Form map fills in.

On the Harbor sheet:

- **Rows 11–34** are the trial balance. E is an agent classifying each account
  onto a form line, following the firm's guide (`guide/mapping.md`) and held
  to its list of categories. "Needs review" is one of them.
- **G–I** build the return with `sumif` over those categories.
- **K–L** are facts an agent reads from the client's documents: prior-year
  AAA, W-3 wages, officer pay, tax depreciation, ownership, distributions.
- **The checks under them** are formulas: the trial balance balances, wages tie
  to the W-3, book-to-tax reconciles, distributions stay within AAA and in
  proportion to ownership, and the K-1s add up.
- **B3** says whether the return is ready, and **B4** is an action that writes
  the filled draft PDF into `returns/`. It refuses until B3 reads ready.

Select E11:E34 and L11:L20 and Run Selection. Then, for each other client,
right-click the Harbor tab → Duplicate Sheet…, name it (Pinecrest, Lumen,
Northside, Kestrel), set B1 to the client's name and B2 to its folder
(`clients/pinecrest`), select A10 and File → Import CSV… its
`trial-balance-2026.csv`, then run its agent cells. The Clients sheet fills in
as each sheet does.

Each client but Harbor has one planted problem:

- **Pinecrest** pays its owner too little for what it distributes, so
  "Officer pay looks reasonable" reads REVIEW.
- **Lumen**'s books carry a family trip the client hopes is deductible. Its own
  email says otherwise, and it belongs in distributions.
- **Northside** keeps the owner's salary in the general wages account, so
  "Officer pay ties to the W-3" reads CHECK until you reclassify it in the
  adjusting-entry rows (35–36).
- **Kestrel** paid its shareholders out of proportion to their ownership.

The agents run on your default model. The recorded version of this demo ran
them on Opus: add `, model='opus'` to the E and L prompts to do the same. The
answer key is `demos/scorp/expected.json` in this repository. The form is a draft and says
so on every page; this demonstrates the board's structure and is not tax
advice.

## research: a literature matrix with a synthesis cell

Five paper abstracts down column A, five questions across; filling the table
does the reading. `synthesis` is an agent cell given the full table as context
(`{A1:F6}`) that writes the related-work paragraph from it. Run it after the
table fills, and re-run it after you correct a cell.

## done-means-green: sessions that can't finish red

Four tasks in a small Python library (`tally`), one per row. Open Session on a
row's B cell starts Claude Code on that task, live beside the grid. E runs the
task's acceptance test whenever the code changes, and F reads `holds` or
`STALE`.

The point is the Stop hook in `.claude/settings.json` (`recalc hook stop`).
When a session tries to finish, the hook writes its session id and its last
message into C and D, waits for the row's test to settle, and reads F. While F
says `STALE`, the session isn't allowed to stop: it gets the failing test's
last line and keeps working. After three tries it may stop, and F is flagged
for you. `finished` and `held_back` count the rows.

Sessions you start yourself in a terminal are held the same way if they run
in a folder named after the task (`git worktree add wt/parse`, then `claude`
in `wt/parse`), with `recalc` on your PATH. Codex sessions are held by
`.codex/hooks.json` once you've opened `codex` there and trusted the hook.

## claims-ledger: four agents, one repository

The same four tasks, this time with four agents working at once. Run Selection
on B2:B5 starts them in the background (each asks once to edit files; allow it
for the run), and each replies `done` or `blocked:` when it stops. Beside each
claim, C runs that task's acceptance test whenever the code changes, and E
reads `holds` while the claim and the test agree, `STALE` when the agent said
done and its test fails now.

What to watch: the invoice and export tasks both lean on `tally.money.fmt`. If
one agent changes it for the invoice, the other's finished export can break
without either agent noticing, and its row turns `STALE` on its own, long
after that agent last ran the tests. `tests_untouched` checks that no agent
edited the tests to get a pass, and `suite_green` reads the whole suite.

## flaky-hunt: prove the fix

Four suspect tests from a suite that fails about one run in three. B runs each
test 50 times on the code as it was and keeps the count. C is an agent naming
the likely cause, held to a fixed set (timing, test order, shared state,
randomness, environment, not flaky). D is a fix session: Open Session on it,
and its Stop hook won't let it finish until the test passes all 50 runs again.

E re-runs the 50-run loop whenever `shipit/` changes, so the evidence updates
as the agent works. F turns "passed 50 times" into a number: with no failures
in n runs, the rule of three bounds the failure rate at about 3/n with 95%
confidence, so it reads "under 6%". G is `true` when a test that failed before
fails none of the runs now. Verify G once you've read the fix; `proven` counts
what you've verified and `spend` sums what the cause-finding cost.

## beads-board: beads decides what's next, the board decides what's done

A [beads](https://github.com/steveyegge/beads) issue list kept in sync as rows
(`bd list --json # rows: A2 key=id …`), so a new bead gets a row and a closed
one updates. `tools/bd` stands in for `bd`; delete `python3 tools/` from the
commands to use the real one on your own beads.

D names each bead's test and E the bead it comes after. F runs the test
whenever the code changes, and G says whether it holds. H is readiness by
checks, not by closes: a bead is ready when the bead it follows *holds*. I
catches a bead that beads says is closed while its test fails. J is a session
for the bead, held by its Stop hook until G holds. L closes the bead in beads,
and refuses until G holds and you've verified it.

As shipped, an agent closed `tl-7f3.1` with its test still failing. `bd ready`
offers `tl-7f3.2` on the strength of that close; the board says it waits.

## review-lanes: which AI reviewer is worth keeping

Open pull requests as rows, kept in sync with the PR list (`tools/gh` stands
in for the GitHub CLI; delete `sh tools/` from `prs` and D2 to use the real
`gh` in your repository). A merged PR keeps its row, greyed, marked gone in K.
D holds the diff, and E and F are facts no model should judge: lines changed
and tests removed.

G, H and I are review lanes, security, tests and design, each an agent held to
`issue` or `ok` and citing the diff line its answer rests on. J is the verdict:
changes needed, ready, or in review. Run Selection on G2:I7, then go down the
answers with Review Answers (⌥⌘R): verify the ones you agree with, flag the
ones you don't. Each lane's precision is the share of its `issue` answers you
verified, so after a few weeks of PRs the board says which lane is noise.

## release-gate: checks, a sign-off, and a tag that waits

What must be true before acme 2.4.0 ships, one row per check:

- CI on main, re-checked every minute;
- the changelog has a section for the version;
- every version string agrees;
- every migration can be rolled back;
- someone smoke-tested staging (a cell a person verifies);
- release notes an agent drafts and a person verifies.

C says whether each row passes, `ready` is all of them, and the `tag` action
refuses to run until `ready` is true. Two rows fail as shipped:
`src/acme/__init__.py` still says 2.3.1, and migration 007 has no down step.
Fix either file and its row turns green on its own. The same board is a CI
gate: `recalc check board.recalctemplate --cell ready` exits nonzero until it
reads true.

## cve-triage: advisories against the code that ships

Eight scanner advisories against a small Python service, `ledgerline`. Only
`ledgerline/` ships; `tests/`, `tools/` and `requirements-dev.txt` don't.

B through E are facts a command establishes (`checks/advisory.py`): the
package, the installed version, where it's pinned, and whether that version is
in the affected range, compared as numbers, so a version exactly at the patch
boundary reads No. H finds the calls to the vulnerable function in
`ledgerline/` (`checks/reach.py`), following import aliases, and re-runs when
the code changes. The agent cells (F, G, J–L) give reachability, the line that
shows it, severity in context, the action, and a justification for anything
accepted. I reads `look` where the agent's reachability and the call-site
check disagree, so you know which rows to read first.

The traps: an import alias that hides a real call, a version at the boundary,
a vulnerable function used only in tests, a dependency only the linter pulls
in, one advisory that's another under a second feed's id, a call site that
passes the safe flag, and an upgrade that would break the code. Select F2:L9
and Run Selection. The answer key is `demos/cve-triage/expected.json` in this
repository.

## bench: benchmarks that keep score

`bench.py` is a small dependency-free suite. Each case's cell re-runs when the
file changes (`# watch: bench.py`); the total re-runs every minute
(`# every: 60000`). Every run lands in the cell's history, so the sparkline
shows drift and `median_ms` reads the median across runs. `regressed` turns
red when the total is 15% over baseline. `explain` asks an agent to read the
script and the git log and say why; it only runs when you run it.

Try it: edit `bench.py` to make `case_sort` sort the list twice, save, and
watch the sort row and the total move without touching anything.

## models: the same tasks, two agents, a score

Column C asks Claude, column D asks Codex, the same task per row. Formulas
grade exact answers against the expected column; the last row is free text,
so an agent judges it. `claude_score` and `codex_score` sum up. Add rows to
grow the eval; fill C and D down to cover them.

## sec-questionnaire: answers checked against the Terraform

A customer's security questionnaire (twelve questions from a regional health
system) answered by a software vendor from its own policies, reports, and
infrastructure. The agent cells give the answer (Yes, No, Partial, N/A), an
explanation the customer can read, the passage it rests on, which
configuration claim it makes, whether a person must decide something before it
goes out, a second agent's review, and the date of time-bound evidence like a
pen test.

F tests the configuration claim the agent named against the Terraform and
exports in `infra/` (`checks/config.py`), and re-runs when `infra/` changes. G
reads `CONTRADICTED` for a Yes beside a failing configuration. A passing check
doesn't make the whole answer true, though: daily backups pass, but nothing
shows a restore test. `ready` says "ready to send" only when nothing is
contradicted, nothing is Not found, and the reviewer agrees with every answer.

The traps: one storage bucket without encryption under a policy that says
everything is encrypted, a break-glass login without MFA, a superseded policy
with a different recovery time, a pen test more than a year old, and a
question nothing on file answers. Select B2:J13 and Run Selection. The answer
key is `demos/sec-questionnaire/expected.json` in this repository.

## soc2-evidence: controls re-performed by commands

Ten SOC 2 Type II controls for a payroll company a month from fieldwork,
tested over six months of exports (pull requests, change tickets, HR records,
the identity provider, vulnerability scans, access reviews).

The counts are commands (`checks/controls.py`) that re-perform each control's
test over the files in `evidence/`, applying the policies' own allowances (a
Standard change, a cancelled ticket, a retest within 7 days), so every number
can be run again for the auditor. The judgement calls are agent cells: which
files test the control, whether a deviation is one the policy allows, and
whether the evidence covers the whole period. J is the result, computed from
the counts. K is a second agent re-performing the test from the files without
seeing the counts, and L says whether it agrees. `evidence_untouched` checks
that nothing under `evidence/` changed.

The traps are the ones auditors find: an admin-bypass merge in the period and
another just outside it, a deactivation 7 business days after a termination,
an access review signed 50 days after quarter end, deprovisioned accounts that
aren't in scope, and a training deadline that is late on day 31, not day 30.
Select C2:K11 and Run Selection. The answer key is
`demos/soc2-evidence/expected.json` in this repository.

## Making your own

Any of these is a pattern: things down a column, questions across a row, one
prompt in the top-left cell with `{$A2}` and `{B$1}` anchors, filled across
the block. `File → Import Files as Rows` puts a folder's files down a column.
Save a board you like with `recalc save my-board.recalc --note "…"`.
