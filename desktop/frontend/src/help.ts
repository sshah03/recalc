// The teaching surfaces: the function catalog (shared with autocomplete)
// and the cheatsheet panel. Static content, no state.

export interface FuncDoc {
  name: string;
  sig: string;
  what: string;
}

// Every Func::signature in src/expr.rs, with descriptions written by hand.
export const FUNCTIONS: FuncDoc[] = [
  { name: "sum", sig: "sum(range or value, ...)", what: "add the numbers in ranges and values" },
  { name: "avg", sig: "avg(range or value, ...)", what: "average of the numeric values" },
  { name: "count", sig: "count(range or value, ...)", what: "how many cells are non-empty" },
  { name: "min", sig: "min(range or value, ...)", what: "smallest number" },
  { name: "max", sig: "max(range or value, ...)", what: "largest number" },
  { name: "median", sig: "median(range or value, ...)", what: "middle number" },
  { name: "countif", sig: "countif(range, criterion)", what: "count cells matching '>5', 'x', '<>x'" },
  { name: "countifs", sig: "countifs(range, criterion, range, criterion, …)", what: "count rows where every range matches its criterion" },
  { name: "sumif", sig: "sumif(range, criterion[, sum_range])", what: "sum where the criterion matches" },
  { name: "round", sig: "round(number[, digits])", what: "round half away from zero" },
  { name: "abs", sig: "abs(number)", what: "distance from zero" },
  { name: "if", sig: "if(cond, then, else)", what: "pick a branch" },
  { name: "and", sig: "and(cond, ...)", what: "true when every condition is" },
  { name: "or", sig: "or(cond, ...)", what: "true when any condition is" },
  { name: "not", sig: "not(cond)", what: "negate a condition" },
  { name: "concat", sig: "concat(value, ...)", what: "join values into text" },
  { name: "len", sig: "len(text)", what: "character count" },
  { name: "lower", sig: "lower(text)", what: "lowercase" },
  { name: "upper", sig: "upper(text)", what: "uppercase" },
  { name: "trim", sig: "trim(text)", what: "strip surrounding whitespace" },
  { name: "lookup", sig: "lookup(key, search_range, result_range)", what: "the value beside a match" },
  { name: "match", sig: "match(key, range)", what: "position of a match (1-based)" },
];

const KEYS: Array<[string, string]> = [
  ["⌘↑ ⌘↓ ⌘← ⌘→ (or ⌥ and an arrow)", "to the edge of the filled block, or the next filled cell; with ⇧, select that far"],
  ["Home · End · ⌘Home · ⌘End", "the start or the filled end of the row; A1 or the last filled cell"],
  ["Page Up · Page Down", "a screenful up or down"],
  ["⇧Space · ⌃Space · ⌘A", "select the row, the column, or every filled cell"],
  ["⌥↩ while editing", "a line break inside the cell"],
];

const KINDS: Array<[string, string]> = [
  ["42", "a value, typed as is"],
  ["= {failed} == 0", "a formula over other cells"],
  ["= {name} & ' Inc'", "& joins text; $1,200, 1,000 and 5% count as numbers"],
  ["$ cargo test 2>&1", "a command; output is the value, re-runs when inputs change"],
  ["! git commit -am {msg}", "an action with side effects; runs only when you run it"],
  ["@claude('prompt', {A1:B3})", "an agent call; like ! it runs only when you run it"],
];

const REFS: Array<[string, string]> = [
  ["{name} or {B2}", "another cell's value; makes it a dependency"],
  ["{A1:B3}", "a rectangular range: a function argument in a formula, one word per cell in a command"],
  ["{Notes!A2}, {Notes!A2:C5}, {'My Notes'!A2}", "a cell or range on another sheet; an unqualified {A2} means the cell's own sheet. Tabs under the grid switch, + adds a sheet"],
  ["{plan_1, plan_2, B4}", "a list of cells, used in a formula wherever a range is"],
  ["{B2.code}, {B2.err}, {B2.dur}, {B2.cost}", "a cell's last run: exit status, stderr, milliseconds, dollars. A pipeline's status is its last command's: `cmd | tail -1` reads 0 even when cmd failed"],
  ["{B2.mark}, {B2.by}, {B2.cited}", "its review: verified or flagged, who wrote it, whether its citation was found; = countif({B2:B9.mark}, 'verified')"],
  ["{$A2}, {B$1}, {$A$1}", "anchors: fill keeps the $ axis fixed (F4 cycles them while editing)"],
  ["{raw:file}", "path to the cell's backing file, for big values"],
  ["{file:src/main.rs}", "a repo file's path; a $ cell re-runs when it changes"],
  ["{glob:src/**/*.rs}", "every matching path; re-runs as files change or appear"],
  ["results = A2:A20  →  = sum({results})", "a named range: any cell whose value is a range"],
];

const ANNOTATIONS: Array<[string, string]> = [
  ["$ cargo test 2>&1  # watch: src/**/*.rs", "re-run whenever a matching file changes"],
  ["$ date +%H:%M:%S  # every: 5000", "re-run on a period (milliseconds)"],
  ["$ gh pr list --json number,title  # rows: A2 key=number cols=number,title", "the output as rows from A2, matched by key: new ones appended with the row above's formulas, gone ones greyed, answers kept"],
  ["$ pytest -q  # cwd: backend", "run in another directory"],
  ["$ git log -1 --format=%h  # watch: .git/HEAD", "git-aware: re-run on commit or checkout"],
];

const AGENTS: Array<[string, string]> = [
  ["@claude('summarize {failed}', {A1:A20})", "runs Claude Code in the background, connected to this board; its answer is the value"],
  ["@codex('fix the flaky test')", "the same with Codex; the agent can read and write cells while it runs"],
  ["@claude('summarize {A2}', model='haiku')", "the same on another model: an alias from [models] in config.toml or the agent's own name for it; {B2.model} reads the model that ran"],
  ["@claude('{B$1} Document: {$A2}')", "a document grid: fill this across a block, then Run Selection"],
  ["The quote mark (”) on a cell", "the agent attached the passage its answer rests on; the inspector shows it, Show Source opens the document"],
  ["Answer Type… on a cell", "yes/no, a date, a number, one of a set; an agent's answer that doesn't fit is sent back. Add Question Column sets it, or set it by hand"],
  ["Typing in a typed cell", "it offers what the cell takes: its options or Yes and No (▾ on the cell lists them), or the date you typed (\"oct 3\") as YYYY-MM-DD, with a calendar; a value of yours that doesn't fit is kept, and the inspector says what the cell takes"],
  ["Needs You (⌘J)", "questions, suggestions, changed inputs, errors, flags in one list; digits answer a question; Later puts a row aside"],
  ["Run Selection (⇧⌘↩)", "run every command, action, and agent cell in the selection; a few agents run at once (the limit is in the board's settings) and the rest wait their turn"],
  ["A board's scripts", "its ! cells: ⌘K lists them (Run normalize_dates), and Edit Script in the Inspector opens the script a cell runs; a script reads cells as arguments ({B2:B50}) or with recalc get, and writes a block back with recalc import - --at C2"],
  ["Ask the Driver to Answer (Run ▾, or right-click)", "the Driver answers the selected agent cells itself, in its own conversation: quicker and cheaper for simple ones; each keeps its prompt, so Run asks the cell's own agent later. Run Selection keeps answers independent"],
  ["Open Session (⇧⌘T) on an agent cell", "the agent in a terminal you can watch and type into; a cell named after it with _status on the end says whether it is working or waiting"],
  ["[inputs changed]", "a session's inputs changed after it started; Tell Agent About Changed Inputs types the changes into its terminal"],
  ["Resume session on an agent cell", "after a restart, continue its last conversation; More → New Conversation starts over"],
  ["Open in Terminal (⌥⌘T) on an agent cell", "the same conversation in a terminal window: joins the live session (it outlives this window), or picks up a finished run"],
  ["Driver (⇧⌘D)", "one agent in the side panel that works the board for you: fills cells, starts other agents, points you at what needs a look; to seat Codex instead of Claude Code, set driver to codex in the board's settings"],
  ["review = @claude('…')  →  {review}", "reference the answer; {review.code} is its exit status, {review.mark} whether a person verified it"],
  ["The board's settings: .recalc/config.toml", 'add your own agents (myagent = "mytool {task}"), set how long a run may take and how many agents run at once'],
  ["claude mcp add recalc -- recalc mcp", "wire an interactive Claude Code to this board; Agent → Connect an Agent gives the app's full path"],
];

const REVIEW: Array<[string, string]> = [
  ["Review Answers (⌥⌘R)", "go through the unverified answers one at a time with the source open: V verifies, F flags, S skips, Esc stops"],
  ["Space on a cell", "the whole answer in a card; Space again closes it"],
  ["Help → Document Grids Guide", "the long form: starting a table, reading answers, signing off, exporting"],
  ["File → New Board… → Document grid from a folder", "a folder of documents down column A, then your first question column"],
  ["Add Question Column… (⇧⌘Q)", "a question and the kind of answer you want (yes/no, date, amount, a pick), asked of every row about the column you pick: its documents, or whatever it holds"],
  ["Run Column (⌥⌘↩)", "run the column below the header; the header counts what's still running"],
  ["Show source (⌥⌘S)", "the document behind the selected cell, its cited passage highlighted, beside the grid"],
  ["Verify (⌘Y)", "you signed off: agents' writes become suggestions; fill and Run Selection skip it"],
  ["Next Unverified Answer (⌥⌘↓) · Verify and Next (⌥⌘Y)", "review mode in two keys: jump to the next answer nobody has signed off on; sign off and jump"],
  ["Flag… (⇧⌘Y)", "mark a cell to come back to, with a note"],
  ["select a block", "the inspector counts answers, verified, flagged, running, queued"],
  ["View → Show Flagged / Unverified / Errors / Running Rows", "a review filter; Show All Rows brings them back"],
  ["Filter to This Value · Filter Column…", "only the rows whose cell in a column holds a value, or one of several; filters on different columns combine; click the status bar to clear"],
  ["Paste Values (⇧⌘V)", "what you copied, as plain values, never the formula or agent behind it"],
  ["View → Wrap Text (⌥⌘W)", "long answers on three lines; hover any cell for its full value"],
  ["View → Freeze Panes at Selection", "keep the header row and the document column in view"],
  ["Find (⌘F)", "every match tinted; Enter steps through them"],
  ["Stop (⌘.) · Stop All Runs (⇧⌘.)", "kill a run; the cell keeps its last value"],
  ["Copy as Markdown Table (⇧⌘C)", "the selection, first row as header, ready for a memo"],
  ["Format (⌘1, or Format at the toolbar's end) · ⌘B · ⌘I · ⇧⌘L/E/R · ⌘\\", "the side panel's Format view: bold, italic, alignment, wrap, a number preset with its decimals and currency, a fill, and highlights that fill a cell by its value (\"fail\" in red); ⌘\\ clears"],
  ["123 in the toolbar", "number presets, each showing this cell's value the way it would look"],
  ["right-click a column header → Sort A → Z", "select several rows to sort just those; numbers, then text, blanks last; row 1 stays; references follow their rows"],
  ["right-click a row or column header", "insert or delete rows and columns; references follow their cells"],
  ["File → Import Files as Rows", "a folder's files down a column, one document per row"],
  ["File → Export Excel Workbook…", "values with formats; every citation and mark becomes a cell note"],
];

// ---- the document-grids guide: the long form, for people running a review ----

const GUIDE: Array<[string, string[]]> = [
  [
    "What a document grid is",
    [
      "Documents down column A, one question per column header, an agent's answer in every cell. Each answer is an ordinary cell you can reference, sort, filter, format, and sign off on.",
      "Each answer can carry a citation: the file and the passage the agent relied on. A cell with one shows a small closing quote mark; the inspector shows the passage, and Show Source opens the document beside the grid with the passage highlighted.",
    ],
  ],
  [
    "Starting one",
    [
      "File → New Board…, then Document grid from a folder, asks for a folder. Its files land down column A under a Document header, then the question dialog opens.",
      "Add Question Column… (⇧⌘Q) takes a question, the kind of answer you want, and the agent to ask. The answer can be a sentence, yes/no, a date, a number, an amount, one of a fixed set, or a quote. Ask about picks the column each row's question is about: the documents (picked for you when a column holds them), or any other column, such as a company name, which the agent treats as the row's subject. It writes the header, a prompt in every row with something in that column, and the matching number format. Run now runs them; Run Column (⌥⌘↩) runs a column again later.",
      "Pick from the library or write your own; Save to my library keeps a column for the next table. The shared instructions cell holds the text every prompt starts with; editing it changes what the agent is told for every row.",
      "A row can name several files, comma-separated. The agent reads them together; the instructions say a later version controls over an earlier one.",
    ],
  ],
  [
    "Reading the answers",
    [
      "Select any cell to read its answer in full in the inspector, with the citation under it. The Source tab in the side panel follows your selection: a cited cell shows its passage; any other cell in a row shows that row's document.",
      "Not found is the agent saying the document doesn't address the question. The block summary counts them separately from answers, so a column of Not found stands out.",
      "Select a block for the summary: answered, not found, cited, running, queued, errors, verified, flagged. Column headers count what is still running in them.",
    ],
  ],
  [
    "Signing off",
    [
      "Verify (⌘Y) a cell you have checked. Agents can no longer overwrite it (their later writes become suggestions), and Fill and Run Selection skip it, so re-running a column never undoes your work.",
      "To clear a table: Next Unverified Answer (⌥⌘↓) jumps to the next agent answer nobody has signed off on, in reading order; Verify and Next (⌥⌘Y) signs the one you are on and jumps. When every answer is verified, it tells you.",
      "Flag (⇧⌘Y) a cell to come back to, with a note in whatever words your team uses. View → Show Flagged Rows, Show Unverified Rows, and Show Rows with Errors narrow the table to what still needs you.",
      "Agents can flag a cell too when they are unsure, but only you can verify.",
      "The Needs You list (⌘J) in the side panel gathers everything waiting on you, oldest first, with how long each has waited: a question an agent asked, a suggestion on a cell you verified, a session whose inputs changed, an error, a flag. Press a question's option number to answer it, or use the row's own buttons. A row you can't act on yet can be put aside for an hour, until tomorrow, or until the cell changes; it waits on a shelf under the list that shows when each comes back, and comes back on its own.",
      "In a column with an answer type, every answer has to fit it. When an agent writes prose where a date or a Yes/No belongs, the answer is sent back with the type asked for and the agent answers again; a run that prints something that doesn't fit turns red with the answer in its console and never becomes a value your formulas use. Cell → Answer Type… sets the answer type on any cell by hand.",
    ],
  ],
  [
    "Getting it out",
    [
      "Copy as Markdown Table (⇧⌘C) for a memo or a chat. Export CSV for values only. Export Excel Workbook keeps number formats, bold, wrap and fills (a highlight as the fill it gives the value then), freezes the header row, and turns every citation and mark into a cell note, so the evidence travels with the table.",
    ],
  ],
  [
    "For contract review",
    [
      "Put the target's contracts down column A. Group an agreement with its amendments in one row so the agent reads them together. Typed columns keep answers comparable: Change of control as a fixed choice, Liability cap as text, Effective date as a date.",
      "Columns need not read the documents. A risk rating column can be an agent prompt over the answers in other columns ({F2}, {G2}), re-running when they change; a count like =countif({F2:F40}, 'Consent required') gives the deal team its number.",
      "The Legal due diligence example (File → New Board…, under Documents) is a worked example of all of this.",
    ],
  ],
  [
    "For document intake and finance",
    [
      "Forms and statements down column A, amounts as currency columns, then formulas over them: lookup a total by form type, sum a column, compare against a threshold. The tax demo shows the reconciliation pattern.",
    ],
  ],
  [
    "For research and reading",
    [
      "Papers or notes down column A; method, sample size, main finding as columns; a final agent cell given the whole block as context writes the synthesis. The research demo is the template.",
    ],
  ],
];

/** The document-grids guide (Help → Document Grids Guide). */
export function reviewGuide(): HTMLElement {
  const box = document.createElement("div");
  box.className = "guide";
  for (const [title, paras] of GUIDE) {
    const h = document.createElement("h3");
    h.textContent = title;
    box.appendChild(h);
    for (const text of paras) {
      const p = document.createElement("p");
      p.textContent = text;
      box.appendChild(p);
    }
  }
  return box;
}

function section(sheet: HTMLElement, title: string, rows: Array<[string, string]>) {
  const h = document.createElement("h3");
  h.textContent = title;
  sheet.appendChild(h);
  for (const [example, what] of rows) {
    const row = document.createElement("div");
    row.className = "ex";
    // Typed syntax renders as copyable code; menu items and keys render as words.
    const typed = /^[@=$!{\d"'\[]|^[a-z_]+\(|[{}]| = |^claude mcp|^\.recalc/.test(example);
    const code = document.createElement(typed ? "code" : "span");
    if (!typed) code.className = "key";
    code.textContent = example;
    const note = document.createElement("span");
    note.className = "what";
    note.textContent = what;
    row.append(code, note);
    if (!typed) {
      sheet.appendChild(row);
      continue;
    }
    row.title = "Copy this";
    row.onclick = () => {
      void navigator.clipboard?.writeText(example).catch(() => {});
      note.textContent = "copied";
      note.classList.add("copied");
      setTimeout(() => {
        note.textContent = what;
        note.classList.remove("copied");
      }, 900);
    };
    sheet.appendChild(row);
  }
}

/** The cheatsheet body (⌘/ and Help → Cheatsheet). Click a line to copy it. */
export function cheatsheet(): HTMLElement {
  const sheet = document.createElement("div");
  sheet.className = "sheet";
  // Review first: most readers open this to check answers, not to build.
  section(sheet, "Document grids", REVIEW);
  section(sheet, "Agents", AGENTS);
  section(sheet, "Moving and selecting", KEYS);
  section(sheet, "Cell kinds: the first character decides", KINDS);
  section(sheet, "References", REFS);
  section(sheet, "Trailing annotations (watch and every, $ cells only)", ANNOTATIONS);
  section(
    sheet,
    "Functions",
    FUNCTIONS.map((f) => [f.sig, f.what] as [string, string]),
  );
  return sheet;
}
