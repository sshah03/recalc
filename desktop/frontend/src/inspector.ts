// The inspector: details and actions for the selected cell, or a summary and bulk actions for a block.

import { renderProse } from "./prose.js";
import { a1Coord, a1Name, rectLabel } from "./a1.js";
import type { MenuEntry } from "./chrome.js";
import { showContextMenu } from "./chrome.js";
import { firstLine, valueClass } from "./grid.js";
import { sparkline } from "./spark.js";
import { board } from "./state.js";
import * as api from "./tauri.js";
import { attachSession, closeTerminal, openTerminal, openTerminals, showTerminal, showingTerminal, terminalOpen } from "./terminal.js";
import type { Cell } from "./types.js";
import { agentProduct } from "./types.js";
import { actorShort, API_PRICES, costLabel, scriptPath, elapsedLabel, isAgent, isNotFound, isRunnable, kindWord, neverRun, sessionWord, answerTypeLabel, stateKind, stateWord, tokensLabel, permissionAsk, permissionRisks } from "./types.js";

export interface InspectorHandlers {
  /** The last error or command result to show, if any. */
  flash(): { text: string; error: boolean } | null;
  review(): { at: number; total: number; name: string } | null;
  endReview(): void;
  runWithConfirm(cell: Cell): void;
  runSelection(): void;
  setMark(kind: "verified" | "flagged" | null): void;
  flagWithNote(): void;
  deleteSelected(): void;
  select(name: string, end?: string | null): void;
  run(p: Promise<unknown>): void;
  formatSelection(): void;
  showSource(cell: Cell): void;
  addQuestionColumn(): void;
  answerTypeSelection(): void;
  runColumn(): void;
}

const $ = (id: string) => document.getElementById(id)!;
let handlers: InspectorHandlers;
/** Resolved commands, keyed by source and input values, to skip IPC on unrelated deltas. */
const consoleCache = new Map<string, { key: string; info: api.ConsoleInfo | null }>();

export function initInspector(h: InspectorHandlers) {
  handlers = h;
}

/** Re-renders must not replace an open terminal. */
export function inspectorBusy(): boolean {
  return terminalOpen();
}

/** Grow the inspector to half the window while a terminal is open; restore it on close. */
let heightBefore: string | null = null;
function makeRoom() {
  const root = document.documentElement.style;
  if (heightBefore === null) heightBefore = root.getPropertyValue("--inspector-h");
  const target = Math.max(320, Math.round(window.innerHeight * 0.5));
  root.setProperty("--inspector-h", `${target}px`);
}
function giveRoomBack() {
  if (heightBefore === null) return;
  const root = document.documentElement.style;
  if (heightBefore) root.setProperty("--inspector-h", heightBefore);
  else root.removeProperty("--inspector-h");
  heightBefore = null;
}

async function closeTerminalView() {
  await closeTerminal();
  giveRoomBack();
  renderInspector();
}

export function renderInspector() {
  const title = $("inspector-title");
  const body = $("inspector-body");
  body.replaceChildren();
  reviewBar(body);
  const r = board.selectionRect();
  const names = board.selectedNames();

  if (names.length === 0) {
    title.textContent = "Inspector";
    renderUnrun(body);
    if (board.all.size === 0) {
      body.appendChild(hint("To start, choose File → New Board… for a blank board, a document grid or an example, or tell the Driver in the side panel what you need."));
      body.appendChild(hint("Or type in any cell: a number or text, = for a formula, @claude('…') to have an agent answer. ⌘K lists every command and ⌘/ opens the cheatsheet."));
    } else {
      body.appendChild(hint("Select a cell to see its answer, where it came from, and what you can do with it."));
    }
    flashLine(body);
    return;
  }
  if (names.length > 1 && r) {
    renderBlock(title, body, r, names);
    flashLine(body);
    return;
  }

  const name = names[0];
  const cell = board.get(name) ?? null;
  // Each title fragment is capitalized: "B2 · Agent · Not run yet · Verified".
  const cap = (t: string) => t.charAt(0).toUpperCase() + t.slice(1);
  const question = !!cell && questionHeader(cell) !== null;
  const parts = [name, question ? "Question" : cap(cell ? kindWord(cell.kind) : "empty")];
  const state = cell ? stateWord(cell) : "";
  if (state) parts.push(cap(state));
  if (cell?.mark) parts.push(cap(cell.mark.kind));
  if (cell?.session) parts.push(cap(sessionWord(cell.session)));
  title.textContent = parts.join(" · ");

  if (!cell) {
    body.appendChild(hint("Empty. Type to set a value; start with = $ ! or @ for a formula, command, action, or agent."));
    flashLine(body);
    return;
  }
  if (!cell.decision && !question) renderActions(body, cell);
  if (cell.decision) {
    renderDecision(body, cell);
  } else if (isRunnable(cell.kind)) {
    renderConsole(body, cell);
  } else if (cell.kind === "expr") {
    renderExpr(body, cell);
  } else if (!renderHeader(body, cell)) {
    renderValue(body, cell);
  }
  if (cell.stale_inputs?.length) renderStaleInputs(body, cell);
  renderSuggestions(body, cell);
  renderCite(body, cell);
  if (!a1Coord(cell.name)) renderUse(body, cell);
  renderDetails(body, cell);
  renderSparkDetail(body, cell);
  flashLine(body);
}

/** Unverified agent cells that have never run. */
export function unrunAgents(): string[] {
  return board
    .everywhere()
    .filter((c) => neverRun(c) && c.mark?.kind !== "verified")
    .map((c) => c.name);
}

/** With nothing selected, offer to select the block of never-run agent cells. */
function renderUnrun(body: HTMLElement) {
  const names = unrunAgents();
  const coords = names.map(a1Coord).filter((co): co is [number, number] => co !== null);
  if (coords.length === 0) return;
  // Count only grid cells, matching what the button selects; named cells are reported separately.
  const n = coords.length;
  const named = names.length - n;
  const box = document.createElement("div");
  box.className = "unrun";
  const line = document.createElement("div");
  line.className = "unrun-line";
  line.textContent =
    `${n} agent cell${n === 1 ? " hasn't" : "s haven't"} run yet. Select them, then Run selection (⇧⌘↩) to fill them in.` +
    (named > 0 ? ` ${named} more among the named cells.` : "");
  const c0 = Math.min(...coords.map((c) => c[0]));
  const r0 = Math.min(...coords.map((c) => c[1]));
  const c1 = Math.max(...coords.map((c) => c[0]));
  const r1 = Math.max(...coords.map((c) => c[1]));
  const from = a1Name(c0, r0);
  const to = a1Name(c1, r1);
  const acts = document.createElement("div");
  acts.className = "acts";
  acts.appendChild(
    button(from === to ? `Select ${from}` : `Select ${from}:${to}`, () => handlers.select(from, from === to ? null : to), "Select the block that holds the agent cells still to run"),
  );
  box.append(line, acts);
  body.appendChild(box);
}

/** Primary actions in front, the rest under More. Buttons vary by cell; their order doesn't. */
function renderActions(body: HTMLElement, cell: Cell) {
  const acts = document.createElement("div");
  acts.className = "acts first";
  const more: Array<MenuEntry | null> = [];
  const k = stateKind(cell.state);
  const busy = k === "computing" || k === "queued";
  const verified = cell.mark?.kind === "verified";
  const flagged = cell.mark?.kind === "flagged";

  if (isRunnable(cell.kind)) {
    if (busy) {
      // No Run while busy: it would supersede the live run and discard its result.
      const b = button(k === "computing" ? (cell.activity ? `${cell.activity}…` : "running…") : "queued…", () => {});
      b.disabled = true;
      b.title = k === "queued" ? `Waiting for a free agent (${board.agents.running} of ${board.agents.max} busy)` : "The result lands in the cell when it finishes";
      acts.appendChild(b);
      acts.appendChild(button("Stop", () => handlers.run(api.stop(cell.name)), "Stop the run; the cell keeps its last value (⌘.)", "danger"));
    } else {
      const label = cell.kind === "action" ? "Trigger" : cell.kind === "agent_call" ? "Run agent" : "Run";
      const title = cell.kind === "action" ? "Run this action once (⌘↩)" : cell.kind === "agent_call" ? "Run the agent in the background; its answer becomes the value (⌘↩)" : "Run the command now (⌘↩)";
      acts.appendChild(button(label, () => handlers.runWithConfirm(cell), title));
    }
    // A cell that runs a script in the board: open the script to read or change it.
    const script = scriptPath(cell);
    if (script) acts.appendChild(button("Edit Script", () => handlers.run(api.openScript(script)), `Open ${script} in your text editor; the cell runs it as saved`));
    if (cell.kind === "agent_call") {
      // With no live session but a conversation on record, Open continues it.
      const resumes = !cell.session && !!cell.session_id;
      acts.appendChild(button(resumes ? "Resume session" : "Open session", () => void openAgentSession(cell),
        cell.session
          ? "Show the agent's live terminal (⇧⌘T)"
          : resumes
            ? "Continue the agent's last conversation in a terminal you can watch and type into (⇧⌘T)"
            : "Start the agent in a terminal you can watch and type into (⇧⌘T)"));
      if (cell.stale_inputs?.length) {
        acts.appendChild(button("Tell agent about changed inputs", () => handlers.run(api.sessionNotify(cell.name)),
          "Type the list of changed inputs into its terminal, as you would"));
      }
      if (cell.session || cell.session_id) {
        more.push({ label: "Open in Terminal", shortcut: "⌥⌘T", run: () => handlers.run(api.sessionHandoff(cell.name, true)) });
      }
      if (cell.session) more.push({ label: "End session", run: () => handlers.run(api.sessionEnd(cell.name)) });
      if (resumes) more.push({ label: "New Conversation", run: () => void openAgentSession(cell, true) });
    } else {
      more.push({ label: "Open terminal", shortcut: "⌘T", run: () => void openCellTerminal(cell) });
    }
    if (more.length) more.push(null);
  }
  const verify = button(verified ? "Unverify" : "Verify", () => handlers.setMark(verified ? null : "verified"),
    verified ? "Take back your sign-off (⌘Y)" : "Sign off on this value; agents can no longer overwrite it (⌘Y)");
  if (!verified && cell.value === "") {
    verify.disabled = true;
    verify.title = "Nothing to sign off on yet: run the cell or give it a value first";
  }
  acts.appendChild(verify);
  more.push(flagged ? { label: "Unflag", run: () => handlers.setMark(null) } : { label: "Flag…", shortcut: "⇧⌘Y", run: () => handlers.flagWithNote() });
  more.push(null);
  more.push({ label: "Format…", shortcut: "⌘1", run: () => handlers.formatSelection() });
  more.push({ label: "Answer type…", run: () => handlers.answerTypeSelection() });
  more.push({ label: cell.series ? "Hide history chart" : "Show history chart", run: () => handlers.run(api.setSeries(cell.name, !cell.series)) });
  more.push(null);
  more.push({ label: "Delete", run: () => handlers.deleteSelected() });
  acts.appendChild(moreButton(more));
  body.appendChild(acts);
}

function moreButton(entries: Array<MenuEntry | null>): HTMLButtonElement {
  const b = button("More", () => {}, "Everything else you can do here");
  b.classList.add("more");
  b.onclick = () => {
    const r = b.getBoundingClientRect();
    showContextMenu(r.left, r.bottom + 4, entries);
  };
  return b;
}

/** Label-value rows for format, answer type, mark and provenance; empty rows are omitted. */
function renderDetails(body: HTMLElement, cell: Cell) {
  const rows: Array<[string, string, string?]> = [];
  const f = cell.format;
  const bits: string[] = [];
  if (f?.bold) bits.push("bold");
  if (f?.italic) bits.push("italic");
  if (f?.align) bits.push(`aligned ${f.align}`);
  if (f?.num) bits.push(f.num === "text" ? "plain text" : f.num === "integer" ? "whole number" : f.num);
  if (f?.fill) bits.push("filled");
  if (bits.length) rows.push(["Shown as", bits.join(", ")]);
  if (cell.answer_type) rows.push(["Answers", answerTypeLabel(cell.answer_type)]);
  if (cell.mark) {
    const when = new Date(cell.mark.at).toLocaleTimeString();
    rows.push([
      cell.mark.kind === "verified" ? "Verified" : "Flagged",
      `by ${actorShort(cell.mark.by)} at ${when}${cell.mark.note ? ` · ${cell.mark.note}` : ""}`,
      `mark-${cell.mark.kind}-text`,
    ]);
  }
  const when = new Date(cell.provenance.at).toLocaleTimeString();
  let by = `${actorShort(cell.provenance.by)} at ${when}${cell.provenance.note ? ` · ${cell.provenance.note}` : ""}`;
  if (cell.def_by && cell.def_by !== cell.provenance.by) by += ` · set up by ${actorShort(cell.def_by)}`;
  // An agent cell the Driver answered itself, rather than its own agent.
  const byDriver = cell.kind === "agent_call" && cell.provenance.by === "agent:driver";
  rows.push([byDriver ? "Answered by" : "Changed by", byDriver ? `the Driver at ${when}${cell.provenance.note ? ` · ${cell.provenance.note}` : ""} · Run asks the cell's own agent` : by]);
  if (cell.session_id) rows.push(["Conversation", `${cell.session_id.slice(0, 8)} · Open in Terminal continues it`]);

  const dl = document.createElement("div");
  dl.className = "details";
  for (const [k, v, cls] of rows) {
    const kEl = document.createElement("span");
    kEl.className = "k";
    kEl.textContent = k;
    const vEl = document.createElement("span");
    vEl.className = `v ${cls ?? ""}`.trim();
    vEl.textContent = v;
    if (k === "Conversation") vEl.title = cell.session_id ?? "";
    dl.append(kEl, vEl);
  }
  body.appendChild(dl);
}

/** A named cell: how to refer to it, and what does. */
function renderUse(body: HTMLElement, cell: Cell) {
  const ref = `{${cell.name}}`;
  const users = board
    .everywhere()
    .filter((c) => c.deps.includes(cell.name) || c.deps.includes(board.full(cell.name)))
    .map((c) => c.name);
  const box = document.createElement("div");
  box.className = "use";
  const line = document.createElement("div");
  line.append("Other cells use it as ");
  const code = document.createElement("code");
  code.textContent = ref;
  const copy = document.createElement("button");
  copy.className = "act small";
  copy.textContent = "Copy";
  copy.title = `Copy ${ref} to paste into a formula, command or prompt`;
  copy.onclick = () => handlers.run(api.copyText(ref).then(() => (copy.textContent = "Copied")));
  line.append(code, " ", copy);
  box.appendChild(line);
  const used = document.createElement("div");
  if (users.length) {
    used.append("Used by ");
    for (const n of users.slice(0, 12)) {
      const chip = document.createElement("button");
      chip.className = "act-cell";
      chip.textContent = n;
      chip.title = `Select ${n}`;
      chip.onclick = () => handlers.select(n);
      used.append(chip, " ");
    }
    if (users.length > 12) used.append(`and ${users.length - 12} more`);
  } else {
    used.className = "hint";
    used.textContent = `Nothing uses it yet. Type ${ref} in another cell (= ${ref} * 2, or inside a command or an agent's prompt), and that cell updates when this one changes. Agents can read and write it by name.`;
  }
  box.appendChild(used);
  body.appendChild(box);
}

/** The cited file, locator and quote, with a button to open it in the source pane. */
function renderCite(body: HTMLElement, cell: Cell) {
  const c = cell.cite;
  if (!c) return;
  const box = document.createElement("div");
  box.className = "cite";
  const head = document.createElement("div");
  head.className = "cite-head";
  const file = document.createElement("span");
  file.className = "cite-file";
  file.textContent = c.where ? `${c.file}, ${c.where}` : c.file;
  const open = button("Show source", () => handlers.showSource(cell), "Open the document beside the grid with this passage highlighted (⌥⌘S)", "small");
  head.append(file, open);
  const quote = document.createElement("blockquote");
  quote.className = "cite-quote";
  quote.textContent = c.quote.trim();
  quote.title = `cited by ${actorShort(c.by)}`;
  box.append(head, quote);
  if (c.found === false) {
    box.appendChild(errLine(`This passage isn't in ${c.file} as quoted. Check the answer against the document before you trust it.`));
  }
  body.appendChild(box);
}

function flashLine(body: HTMLElement) {
  const msg = handlers.flash();
  if (!msg) return;
  if (msg.error) body.appendChild(errLine(msg.text));
  else {
    const el = document.createElement("div");
    el.className = "note-line";
    el.textContent = msg.text;
    body.appendChild(el);
  }
}

// ---- a block ----

function renderBlock(title: HTMLElement, body: HTMLElement, r: [number, number, number, number], names: string[]) {
  const cells = names.map((n) => board.get(n)).filter((c): c is Cell => !!c);
  const count = (f: (c: Cell) => boolean) => cells.filter(f).length;
  const agents = count((c) => c.kind === "agent_call");
  const answered = count((c) => c.kind === "agent_call" && c.value !== "" && stateKind(c.state) === "ok" && !isNotFound(c.value));
  const notFound = count((c) => c.kind === "agent_call" && stateKind(c.state) === "ok" && isNotFound(c.value));
  const cited = count((c) => c.cite !== null);
  const running = count((c) => stateKind(c.state) === "computing");
  const queued = count((c) => stateKind(c.state) === "queued");
  const errors = count((c) => stateKind(c.state) === "error");
  const verified = count((c) => c.mark?.kind === "verified");
  const flagged = count((c) => c.mark?.kind === "flagged");
  const runnable = count((c) => isRunnable(c.kind));
  const typed = count((c) => c.answer_type !== null);
  let cost = 0;
  let tokens = 0;
  let priced = 0;
  for (const c of cells) {
    if (!c.usage) continue;
    tokens += c.usage.input_tokens + c.usage.output_tokens;
    if (typeof c.usage.cost_usd === "number") {
      cost += c.usage.cost_usd;
      priced++;
    }
  }
  const unrun = count((c) => neverRun(c));
  title.textContent = `${rectLabel(r)} · ${names.length} cells`;

  const facts: Array<[string, number, string]> = [
    ["agent cells", agents, ""],
    ["not run", unrun, ""],
    ["answered", answered, "v-true"],
    ["not found", notFound, "v-stale"],
    ["cited", cited, ""],
    ["running", running, "v-computing"],
    ["queued", queued, "v-queued"],
    ["errors", errors, "v-error"],
    ["verified", verified, "mark-verified-text"],
    ["flagged", flagged, "mark-flagged-text"],
    ["with an answer type", typed, ""],
  ];
  const row = document.createElement("div");
  row.className = "facts";
  for (const [label, n, cls] of facts) {
    if (n === 0) continue;
    const f = document.createElement("span");
    f.className = `fact ${cls}`;
    f.textContent = `${n} ${label}`;
    row.appendChild(f);
  }
  if (tokens > 0) {
    const f = document.createElement("span");
    f.className = "fact";
    f.textContent = `${priced ? `${costLabel(cost)} at API prices · ` : ""}${tokensLabel(tokens)} tokens`;
    f.title = `${priced && priced < cells.filter((c) => c.usage).length ? "Some runs here report no price; the cost covers the ones that do." : "What the agent runs here reported."} ${API_PRICES}`;
    row.appendChild(f);
  }
  body.appendChild(row);

  const acts = document.createElement("div");
  acts.className = "acts";
  if (runnable > 0) {
    acts.appendChild(button("Run selection", () => handlers.runSelection(), "Run every command, action, and agent cell here; verified and running cells are skipped (⇧⌘↩)"));
  }
  const allVerified = verified === cells.length && cells.length > 0;
  acts.appendChild(button(allVerified ? "Unverify all" : "Verify all", () => handlers.setMark(allVerified ? null : "verified"),
    allVerified ? "Take back your sign-off on every cell here" : "Sign off on every cell here (⌘Y)"));
  const more: Array<MenuEntry | null> = [
    { label: "Flag…", shortcut: "⇧⌘Y", run: () => handlers.flagWithNote() },
    verified + flagged > 0 ? { label: "Clear marks", run: () => handlers.setMark(null) } : null,
    null,
    { label: "Format…", shortcut: "⌘1", run: () => handlers.formatSelection() },
    { label: "Answer type…", run: () => handlers.answerTypeSelection() },
    agents > 0 ? { label: "Add question column…", shortcut: "⇧⌘Q", run: () => handlers.addQuestionColumn() } : null,
    null,
    { label: "Delete", run: () => handlers.deleteSelected() },
  ];
  acts.appendChild(moreButton(more.filter((e, i, arr) => !(e === null && (i === 0 || arr[i - 1] === null)))));
  body.appendChild(acts);
  if (cells.length < names.length) body.appendChild(hint("⌘D fills down from the first row and ⌘R fills right. A prompt with {B$1} and {$A2} becomes a table when filled both ways."));
}

// ---- one cell ----

/** The agent cells below `cell` if it is a row-1 header over them, else null. */
function questionHeader(cell: Cell): Cell[] | null {
  const co = a1Coord(cell.name);
  if (!co || co[1] !== 1 || cell.kind !== "literal") return null;
  const [, maxRow] = board.extent();
  const below: Cell[] = [];
  for (let row = 2; row <= maxRow; row++) {
    const c = board.get(a1Name(co[0], row));
    if (c?.kind === "agent_call") below.push(c);
  }
  return below.length ? below : null;
}

function renderHeader(body: HTMLElement, cell: Cell): boolean {
  const below = questionHeader(cell);
  if (!below) return false;
  const q = document.createElement("div");
  q.className = "prose question-text";
  q.textContent = cell.value;
  body.appendChild(q);
  const n = (f: (c: Cell) => boolean) => below.filter(f).length;
  const answered = n((c) => stateKind(c.state) === "ok" && c.value !== "" && !isNotFound(c.value));
  const notFound = n((c) => stateKind(c.state) === "ok" && isNotFound(c.value));
  const unrun = n((c) => neverRun(c));
  const verified = n((c) => c.mark?.kind === "verified");
  const bits = [`${below.length} agent cell${below.length === 1 ? "" : "s"} below`, `${answered} answered`];
  if (notFound) bits.push(`${notFound} not found`);
  if (unrun) bits.push(`${unrun} not run`);
  bits.push(`${verified} verified`);
  const typed = below.find((c) => c.answer_type);
  if (typed?.answer_type) bits.push(`answers: ${answerTypeLabel(typed.answer_type)}`);
  body.appendChild(hint(bits.join(" · ")));
  const acts = document.createElement("div");
  acts.className = "acts";
  acts.appendChild(button("Run column", () => handlers.runColumn(), "Run the agent cells under this question; finished ones wait for a second press"));
  body.appendChild(acts);
  return true;
}

function renderValue(body: HTMLElement, cell: Cell) {
  if (cell.value.includes("\n") || cell.value.length > 80) {
    const box = document.createElement("div");
    box.className = "console";
    showHead(box, cell.value, cell.name);
    body.appendChild(box);
  }
}

/** Show at most HEAD_MAX characters; laying out a huge value stalls every selection. */
const HEAD_MAX = 20_000;
export function showHead(el: HTMLElement, text: string, name: string, prose = false) {
  const head = text.length > HEAD_MAX ? text.slice(0, HEAD_MAX) : text;
  if (prose) renderProse(el, head);
  else el.textContent = head;
  if (head.length < text.length) {
    const more = document.createElement("div");
    more.className = "hint";
    const size = text.length >= 1e6 ? `${(text.length / 1e6).toFixed(1)} MB` : `${Math.round(text.length / 1000)} KB`;
    more.textContent = `Showing the first ${HEAD_MAX / 1000} KB of ${size}. The whole of it is in .recalc/cells/${name}; a command reads it with {${name}:file}.`;
    el.appendChild(more);
  }
}

function renderExpr(body: HTMLElement, cell: Cell) {
  const src = document.createElement("div");
  src.className = "source";
  src.textContent = cell.source ?? "";
  body.appendChild(src);
  renderDeps(body, cell);
  const out = document.createElement("div");
  out.className = "result";
  const v = document.createElement("span");
  v.className = valueClass(cell);
  v.textContent = firstLine(cell.value) || stateWord(cell);
  out.append(document.createTextNode("→ "), v);
  body.appendChild(out);
}

function renderDeps(body: HTMLElement, cell: Cell) {
  if (cell.deps.length === 0) return;
  const list = document.createElement("div");
  list.className = "deps";
  for (const dep of cell.deps) {
    const d = board.get(dep);
    const line = document.createElement("div");
    line.className = "dep";
    const nm = document.createElement("a");
    nm.textContent = dep;
    nm.onclick = () => handlers.select(dep);
    const val = document.createElement("span");
    val.className = d ? valueClass(d) : "v-stale";
    val.textContent = d ? firstLine(d.value) || stateFallbackWord(d) : "(empty)";
    line.append(nm, document.createTextNode(" = "), val);
    list.appendChild(line);
  }
  body.appendChild(list);
}

function stateFallbackWord(c: Cell): string {
  return stateWord(c);
}

function consoleKey(cell: Cell): string {
  return (cell.source ?? "") + "\u0000" + cell.deps.map((d) => board.get(d)?.value ?? "").join("\u0000");
}

/** An agent call's parts: `@agent('prompt', {context}…, model='m')`. The
 * prompt ends at its quote followed by `,` or `)`, so context and a model
 * after it aren't read as part of it. */
export function parseCall(source: string): { agent: string; prompt: string; model?: string } | undefined {
  const head = /^\s*@([\w-]+)\(\s*(['"])/.exec(source);
  if (!head) return undefined;
  const q = head[2];
  const start = head[0].length;
  const end = new RegExp(`${q}\\s*[,)]`, "g");
  end.lastIndex = start;
  const close = end.exec(source);
  if (!close) return undefined;
  const rest = source.slice(close.index + 1);
  const model = /\bmodel\s*=\s*(['"])(.*?)\1/.exec(rest)?.[2];
  return { agent: head[1], prompt: source.slice(start, close.index), model };
}

/** An agent cell's prompt as the agent reads it: each short single-line input
 * filled in, a long one (or a range or fact) left as a named chip. The template
 * itself stays in the formula bar. */
function renderAsk(body: HTMLElement, cell: Cell) {
  const call = parseCall(cell.source ?? "");
  if (!call) return;
  const ask = document.createElement("div");
  ask.className = "ask";
  const who = document.createElement("span");
  who.className = "asker";
  who.textContent = `Asks ${agentProduct(call.agent)}${call.model ? ` on ${call.model}` : ""}: `;
  ask.appendChild(who);
  const prompt = call.prompt;
  let at = 0;
  for (const ref of prompt.matchAll(/\{([^{}]+)\}/g)) {
    ask.appendChild(document.createTextNode(prompt.slice(at, ref.index)));
    at = ref.index! + ref[0].length;
    const name = ref[1].replace(/\$/g, "").replace(/:(raw|file)$/, "").trim();
    const dep = /^[\w-]+$/.test(name) ? board.get(name) : undefined;
    const value = dep?.value ?? "";
    const inline = dep && value !== "" && value.length <= 100 && !value.includes("\n");
    const el = document.createElement("a");
    el.className = inline ? "ask-val" : "ask-ref";
    el.textContent = inline ? value : name;
    el.title = inline ? `${name}; select it` : `Select ${name}`;
    const target = name.split(/[:.,\s]/)[0];
    el.onclick = () => handlers.select(target);
    ask.appendChild(el);
  }
  ask.appendChild(document.createTextNode(prompt.slice(at)));
  body.appendChild(ask);
}

function renderConsole(body: HTMLElement, cell: Cell) {
  if (cell.kind === "agent_call") renderAsk(body, cell);
  const key = consoleKey(cell);
  const cached = consoleCache.get(cell.name);
  const box = document.createElement("div");
  box.className = "console";
  const cmd = document.createElement("div");
  cmd.className = "cmd";
  // An agent's command line is long and secondary, so it's folded.
  let fold: HTMLElement | null = null;
  if (cell.kind === "agent_call") {
    fold = document.createElement("details");
    fold.className = "cmd-fold";
    const sum = document.createElement("summary");
    sum.textContent = "The command this cell runs";
    fold.append(sum, cmd);
  } else {
    box.appendChild(cmd);
  }
  const paint = (info: api.ConsoleInfo | null) => {
    cmd.textContent = info ? `$ ${info.command}` : cell.source ?? "";
    const u = cell.usage;
    const working = board.workingFor(cell.name);
    meta.textContent = [
      working !== null ? `working for ${elapsedLabel(working)}` : "",
      // Omit exit code 0 for agents.
      cell.exit_code !== null && !(cell.exit_code === 0 && cell.kind === "agent_call") ? `exit code ${cell.exit_code}` : "",
      cell.duration_ms !== null && working === null ? `took ${cell.duration_ms < 1000 ? `${cell.duration_ms} ms` : elapsedLabel(cell.duration_ms)}` : "",
      u ? `${tokensLabel(u.input_tokens + u.output_tokens)} tokens` : "",
      u && typeof u.cost_usd === "number" ? `${costLabel(u.cost_usd)} at API prices` : "",
      u?.turns ? `${u.turns} turn${u.turns === 1 ? "" : "s"}` : "",
      u?.model ?? "",
      info && info.cwd !== board.dir ? `runs in ${info.cwd}` : "",
      info?.watch ? `re-runs when ${info.watch} changes` : "",
      info?.every ? `re-runs every ${info.every >= 1000 ? `${info.every / 1000} s` : `${info.every} ms`}` : "",
    ]
      .filter(Boolean)
      .join(" · ");
    meta.title = u ? `${u.input_tokens.toLocaleString()} tokens in, ${u.output_tokens.toLocaleString()} out${typeof u.cost_usd === "number" ? `. ${API_PRICES}` : ""}` : "";
  };
  if (cell.value) {
    const out = document.createElement("div");
    // Agent output renders as prose; command output as plain text.
    showHead(out, cell.value, cell.name, cell.kind === "agent_call");
    box.appendChild(out);
  }
  if (cell.stderr) {
    const err = document.createElement("div");
    err.className = "stderr";
    showHead(err, cell.stderr, `${cell.name}`);
    box.appendChild(err);
  }
  if (fold) box.appendChild(fold);
  body.appendChild(box);
  const meta = document.createElement("div");
  meta.className = "meta";
  body.appendChild(meta);
  if (cached && cached.key === key) {
    paint(cached.info);
  } else {
    paint(null);
    void api
      .console_(cell.name)
      .then((info) => {
        consoleCache.set(cell.name, { key, info });
        if (board.selection.active === cell.name) paint(info);
      })
      .catch(() => consoleCache.set(cell.name, { key, info: null }));
  }
  renderDeps(body, cell);
}

function renderStaleInputs(body: HTMLElement, cell: Cell) {
  const box = document.createElement("div");
  box.className = "notice";
  const head = document.createElement("div");
  head.textContent = "Inputs changed since this session started; the agent still has the old values:";
  box.appendChild(head);
  for (const s of cell.stale_inputs ?? []) {
    const line = document.createElement("div");
    line.className = "diff";
    line.textContent = `${s.name}: ${firstLine(s.was) || "(empty)"} → ${firstLine(s.now) || "(empty)"}`;
    box.appendChild(line);
  }
  body.appendChild(box);
}

// Live-session view: an xterm on the cell's persistent PTY, one tab per open
// session. Closing detaches the view; the agent keeps running.
export async function openAgentSession(cell: Cell, fresh = false) {
  makeRoom();
  const body = $("inspector-body");
  // Terminals go by the cell's full name (`Notes!B2`), the grid by its own.
  const id = board.full(cell.name);
  const already = openTerminals().includes(id);
  body.replaceChildren();
  $("inspector-title").textContent = `${cell.name} · session`;
  body.appendChild(sessionTabs(id));
  const acts = document.createElement("div");
  acts.className = "acts";
  acts.appendChild(button("Close (keeps running)", () => closeSessionView(id)));
  acts.appendChild(button("Open in Terminal", () => handlers.run(api.sessionHandoff(cell.name, true)),
    "join this session in a terminal window (tmux); both views show the same agent"));
  if (cell.stale_inputs?.length) {
    acts.appendChild(button("Tell agent about changed inputs", () => handlers.run(api.sessionNotify(cell.name))));
  }
  acts.appendChild(button("End session", () => handlers.run(api.sessionEnd(cell.name).then(() => closeSessionView(id))), "", "danger"));
  body.appendChild(acts);
  const holder = document.createElement("div");
  holder.className = "term-holder";
  body.appendChild(holder);
  if (already) {
    showTerminal(id, holder);
    return;
  }
  const container = document.createElement("div");
  container.className = "term";
  holder.appendChild(container);
  try {
    await attachSession(container, cell.name, fresh);
    showTerminal(id, holder);
    // Rebuild the tab strip to include the new session.
    body.replaceChild(sessionTabs(id), body.firstElementChild!);
  } catch (e) {
    await closeTerminal(id);
    renderInspector();
    body.appendChild(errLine(String(e)));
  }
}

function sessionTabs(current: string): HTMLElement {
  const strip = document.createElement("div");
  strip.className = "session-tabs";
  const ids = new Set([...openTerminals(), current]);
  for (const id of ids) {
    const tab = document.createElement("button");
    tab.className = "session-tab" + (id === current ? " active" : "");
    const name = board.local(id);
    const cell = board.get(name);
    tab.textContent = cell?.session ? `${name} · ${sessionWord(cell.session).replace("session ", "")}` : name;
    tab.onclick = () => {
      const c = board.get(name);
      if (c) void openAgentSession(c);
    };
    strip.appendChild(tab);
  }
  return strip;
}

/** Close one session's view and show the next open one, if any. */
async function closeSessionView(id: string) {
  await closeTerminal(id);
  const next = showingTerminal();
  const cell = next ? board.get(board.local(next)) : undefined;
  if (cell) {
    void openAgentSession(cell);
    return;
  }
  giveRoomBack();
  renderInspector();
}

// Run the cell's resolved command interactively in a PTY in the panel.
export async function openCellTerminal(cell: Cell) {
  const info = await api.console_(cell.name).catch(() => null);
  if (!info) return;
  const body = $("inspector-body");
  body.replaceChildren();
  $("inspector-title").textContent = `${cell.name} · terminal`;
  makeRoom();
  const acts = document.createElement("div");
  acts.className = "acts";
  acts.appendChild(button("Close terminal", closeTerminalView, "Stop the command and put the terminal away"));
  body.appendChild(acts);
  const holder = document.createElement("div");
  holder.className = "term-holder";
  body.appendChild(holder);
  const container = document.createElement("div");
  container.className = "term";
  holder.appendChild(container);
  await openTerminal(container, cell.name, info.command, info.cwd);
}

/** Confirmation for quarantined or destructive-looking commands. */
export function confirmRun(cell: Cell) {
  const body = $("inspector-body");
  const warn = document.createElement("div");
  warn.className = "err";
  warn.textContent = "This command looks like it changes things (a delete, a push, a deploy). Run it anyway?";
  const acts = document.createElement("div");
  acts.className = "acts";
  acts.appendChild(button("Run anyway", () => handlers.run(api.trigger(cell.name)), "", "danger"));
  acts.appendChild(button("Cancel", () => renderInspector()));
  body.append(warn, acts);
}

function renderDecision(body: HTMLElement, cell: Cell) {
  const d = cell.decision!;
  const q = document.createElement("div");
  q.className = "question";
  const who = document.createElement("span");
  who.className = "asker";
  who.textContent = `${actorShort(d.asker)} asks: `;
  const qt = document.createElement("strong");
  qt.textContent = d.question;
  q.append(who, qt);
  body.appendChild(q);
  const what = permissionAsk(d);
  const risks = what ? permissionRisks(what, board.dir ?? "") : [];
  if (risks.length) {
    const tags = document.createElement("div");
    tags.className = "risk-tags";
    for (const r of risks) {
      const t = document.createElement("span");
      t.className = "risk-tag";
      t.textContent = r;
      tags.appendChild(t);
    }
    body.appendChild(tags);
  }
  if (d.held !== undefined) {
    const held = document.createElement("div");
    held.className = "meta";
    held.textContent = `you typed: ${firstLine(d.held)}`;
    body.appendChild(held);
  }
  const rec = d.recommended ?? null;
  d.options.forEach((opt, i) => {
    const row = document.createElement("div");
    row.className = "option" + (i === rec ? " default" : "");
    const num = document.createElement("span");
    num.className = "num";
    num.textContent = String(i + 1);
    const label = document.createElement("span");
    label.textContent = opt.label + (i === rec ? " (recommended)" : "");
    row.append(num, label);
    if (opt.preview?.trimStart().startsWith("{")) {
      // Fold a permission's raw JSON request.
      const fold = document.createElement("details");
      fold.className = "preview-fold";
      fold.onclick = (e) => e.stopPropagation();
      const sum = document.createElement("summary");
      sum.textContent = "The full request";
      const pre = document.createElement("pre");
      pre.className = "preview";
      pre.textContent = opt.preview;
      fold.append(sum, pre);
      row.appendChild(fold);
    } else if (opt.preview) {
      const pre = document.createElement("pre");
      pre.className = "preview";
      pre.textContent = opt.preview;
      row.appendChild(pre);
    }
    row.onclick = () => handlers.run(api.resolveIndex(cell.name, i));
    body.appendChild(row);
  });
  if (d.allow_freetext) {
    const input = document.createElement("input");
    input.className = "field";
    input.placeholder = "…or type your own";
    input.spellcheck = false;
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter" && input.value) handlers.run(api.resolveText(cell.name, input.value));
    });
    body.appendChild(input);
  }
  body.appendChild(hint(rec === null ? "Press the option's number, or click it." : "Press the option's number, or Enter for the recommended one."));
}

function renderSuggestions(body: HTMLElement, cell: Cell) {
  if (cell.pending_suggestions.length === 0) return;
  const box = document.createElement("div");
  box.className = "notice";
  const head = document.createElement("div");
  const verified = cell.mark?.kind === "verified";
  head.textContent = verified
    ? "Suggested changes to this verified cell; adopting or turning them down is up to you:"
    : "Suggestions waiting; the cell's owner decides:";
  box.appendChild(head);
  for (const s of cell.pending_suggestions.slice(-3)) {
    const line = document.createElement("div");
    line.className = "diff";
    line.textContent = `${actorShort(s.from)}: ${firstLine(s.value)}${s.note ? ` · ${s.note}` : ""}`;
    if (verified || !isAgent(cell.def_by ?? "")) {
      line.appendChild(smallButton("Adopt", () => handlers.run(api.setCell(cell.name, s.value))));
      line.appendChild(smallButton("Turn down", () => handlers.run(api.dismissSuggestion(cell.name, s.value))));
    }
    box.appendChild(line);
  }
  body.appendChild(box);
}

function renderSparkDetail(body: HTMLElement, cell: Cell) {
  if (!cell.series || !cell.spark || cell.spark.length < 2) return;
  const wrap = document.createElement("div");
  wrap.className = "meta";
  const s = sparkline(cell.spark, 160, 22);
  if (s) wrap.appendChild(s);
  const range = document.createElement("span");
  range.style.marginLeft = "8px";
  range.textContent = `${Math.min(...cell.spark)}..${Math.max(...cell.spark)} over ${cell.spark.length}`;
  wrap.appendChild(range);
  body.appendChild(wrap);
}

// ---- bits ----

function button(label: string, onClick: () => unknown, title = "", cls = ""): HTMLButtonElement {
  const b = document.createElement("button");
  b.className = `act ${cls}`.trim();
  b.textContent = label;
  if (title) b.title = title;
  b.onclick = () => void onClick();
  return b;
}

function smallButton(label: string, onClick: () => unknown): HTMLButtonElement {
  const b = button(label, onClick);
  b.classList.add("small");
  return b;
}

function hint(text: string): HTMLElement {
  const el = document.createElement("div");
  el.className = "hint";
  el.textContent = text;
  return el;
}

/** Review-pass progress and keys, shown above the selection. */
function reviewBar(body: HTMLElement) {
  const rv = handlers.review();
  if (!rv) return;
  const bar = document.createElement("div");
  bar.className = "review-bar";
  const count = document.createElement("span");
  count.className = "review-count";
  count.textContent = `Reviewing ${rv.at} of ${rv.total}`;
  const meter = document.createElement("span");
  meter.className = "review-meter";
  const fill = document.createElement("span");
  fill.style.width = `${Math.round(((rv.at - 1) / rv.total) * 100)}%`;
  meter.appendChild(fill);
  const keys = document.createElement("span");
  keys.className = "review-keys";
  keys.textContent = "V verify · F flag · J skip · K back";
  const stop = document.createElement("button");
  stop.className = "act small";
  stop.textContent = "Stop (Esc)";
  stop.title = "End the review; what you skipped stays unverified";
  stop.onclick = () => handlers.endReview();
  bar.append(count, meter, keys, stop);
  body.appendChild(bar);
}

function errLine(text: string): HTMLElement {
  const el = document.createElement("div");
  el.className = "err";
  el.textContent = text;
  return el;
}
