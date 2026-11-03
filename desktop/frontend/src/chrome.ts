// Chrome around the grid: toolbar, status strip, overlays and the context menu.
// No board logic: everything it triggers is an action (actions.ts) or an engine call (tauri.ts).

import type { Action } from "./actions.js";
import { rectLabel } from "./a1.js";
import { FILLS, NUM_PRESETS, formatValue, formatApplies, CURRENCIES } from "./format.js";
import type { Format, AnswerType, Highlight } from "./types.js";
import { agentProduct, API_PRICES, costLabel, tokensLabel } from "./types.js";
import { cheatsheet, reviewGuide } from "./help.js";
import { icon } from "./icons.js";
import { needsAction, showTray, working } from "./side.js";
import { agentNote } from "./onboard.js";
import { board } from "./state.js";
import * as api from "./tauri.js";

const $ = (id: string) => document.getElementById(id)!;

/** Error sink once the overlay that started an operation has closed; the app sets it
 * to its error line. Defaults to the console. */
let reportError: (e: unknown) => void = (e) => console.error(e);
export function setErrorReporter(f: (e: unknown) => void) {
  reportError = f;
}

// ---- overlay (one at a time) ----

/** Focus before the overlay opened, restored on close. */
let focusBefore: HTMLElement | null = null;

export function openOverlay(title: string, ...children: HTMLElement[]) {
  const overlay = $("overlay");
  if (overlay.hidden) focusBefore = document.activeElement as HTMLElement | null;
  const modal = document.createElement("div");
  modal.className = "modal";
  modal.setAttribute("role", "dialog");
  modal.setAttribute("aria-modal", "true");
  modal.setAttribute("aria-label", title);
  modal.tabIndex = -1;
  const t = document.createElement("div");
  t.className = "modal-title";
  const name = document.createElement("span");
  name.textContent = title;
  // Esc and a click outside close it too; this is the way out you can see.
  const close = document.createElement("button");
  close.className = "modal-close";
  close.title = "Close (Esc)";
  close.setAttribute("aria-label", "Close");
  close.appendChild(icon("close", 14));
  close.onclick = () => closeOverlay();
  t.append(name, close);
  modal.append(t, ...children);
  overlay.replaceChildren(modal);
  overlay.hidden = false;
  // Trap Tab inside the dialog.
  modal.addEventListener("keydown", (e) => {
    if (e.key !== "Tab") return;
    const stops = [...modal.querySelectorAll<HTMLElement>("button, input, select, textarea, [tabindex='0']")].filter((el) => !(el as HTMLButtonElement).disabled && el.offsetParent !== null);
    if (!stops.length) return;
    const first = stops[0];
    const last = stops[stops.length - 1];
    if (e.shiftKey && (document.activeElement === first || document.activeElement === modal)) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  });
  // Take focus so Esc and keys land here; a dialog that focuses its own field
  // does so afterwards.
  queueMicrotask(() => {
    if (!modal.contains(document.activeElement)) modal.focus({ preventScroll: true });
  });
}

export function closeOverlay() {
  const overlay = $("overlay");
  overlay.hidden = true;
  overlay.replaceChildren();
  const back = focusBefore;
  focusBefore = null;
  if (back?.isConnected) back.focus({ preventScroll: true });
}

export function overlayOpen(): boolean {
  return !$("overlay").hidden;
}

function rowsList(): HTMLElement {
  const list = document.createElement("div");
  list.className = "rows";
  return list;
}

function row(label: string, note: string, onPick: () => void, shortcut?: string, stay = false): HTMLElement {
  const el = document.createElement("div");
  el.className = "row";
  const l = document.createElement("span");
  l.className = "label";
  l.textContent = label;
  const n = document.createElement("span");
  n.className = "note";
  n.textContent = note;
  el.append(l, n);
  if (shortcut) {
    const s = document.createElement("span");
    s.className = "shortcut";
    s.textContent = shortcut;
    el.appendChild(s);
  }
  el.onclick = () => {
    if (!stay) closeOverlay();
    onPick();
  };
  // Keyboard-activatable like a button.
  if (!stay || label) {
    el.tabIndex = 0;
    el.setAttribute("role", "button");
    el.onkeydown = (e) => {
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        el.click();
      }
    };
  }
  return el;
}

// ---- command palette ----

export interface PaletteRow {
  label: string;
  note: string;
  shortcut?: string;
  run: () => void;
}

function paletteRows(actions: Action[], query: string, scripts: () => PaletteRow[]): PaletteRow[] {
  const q = query.toLowerCase();
  // The board's own scripts (its ! cells) first, as Excel lists macros.
  const own = scripts().filter((r) => r.label.toLowerCase().includes(q) || r.note.toLowerCase().includes(q));
  const rows: PaletteRow[] = actions
    .filter((a) => !a.enabled || a.enabled())
    .filter((a) => a.label.toLowerCase().includes(q))
    .map((a) => ({ label: a.label, note: `${a.menu} menu`, shortcut: a.shortcut, run: a.run }));
  if (q) {
    for (const c of board.everywhere()) {
      if (c.name.toLowerCase().includes(q)) {
        rows.push({ label: `Go to ${c.name}`, note: c.value.split("\n")[0], run: () => board.select(c.name) });
      }
    }
  }
  return [...own, ...rows].slice(0, 14);
}

export function openPalette(actions: () => Action[], scripts: () => PaletteRow[] = () => []) {
  const input = document.createElement("input");
  input.className = "field";
  input.placeholder = "Type a command or a cell name";
  input.spellcheck = false;
  const list = rowsList();
  let active = 0;
  let rows: PaletteRow[] = [];
  const render = () => {
    rows = paletteRows(actions(), input.value, scripts);
    if (active >= rows.length) active = 0;
    list.replaceChildren();
    rows.forEach((r, i) => {
      const el = row(r.label, r.note, r.run, r.shortcut);
      if (i === active) el.classList.add("active");
      list.appendChild(el);
    });
  };
  input.addEventListener("input", () => {
    active = 0;
    render();
  });
  input.addEventListener("keydown", (e) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const n = Math.max(1, rows.length);
      active = (active + (e.key === "ArrowDown" ? 1 : n - 1)) % n;
      render();
    } else if (e.key === "Enter" && rows[active]) {
      closeOverlay();
      rows[active].run();
    }
  });
  render();
  openOverlay("Commands", input, list);
  queueMicrotask(() => input.focus());
}

// ---- gallery, recents, cheatsheet, connect ----

export async function openGallery() {
  let failed: unknown = null;
  const templates = await api.templates().catch((e) => {
    failed = e;
    return [];
  });
  const list = rowsList();
  for (const t of templates) {
    list.appendChild(row(...patternCard(t), () => void api.loadTemplate(t.name).catch(reportError)));
  }
  if (failed !== null) list.appendChild(row("", `Could not list the templates: ${String(failed)}`, () => {}, undefined, true));
  openOverlay("Templates to load into this board", list);
}

export async function openRecents(oops: (e: unknown) => void) {
  const recents = await api.recentBoards().catch(() => []);
  const list = rowsList();
  for (const dir of recents) {
    list.appendChild(row(dir, "", () => void api.openBoard(dir).catch(oops)));
  }
  if (recents.length === 0) {
    list.appendChild(row("", "no other boards yet; use File → Open…", () => {}));
  }
  openOverlay("Recent boards", list);
}

export function openGuide() {
  openOverlay("Document grids", reviewGuide());
  $("overlay").querySelector(".modal")?.classList.add("wide");
}

export function openCheatsheet() {
  openOverlay("Cheatsheet", cheatsheet());
}

/** Connect an Agent: which agents resolve on this machine, plus the cheatsheet's
 * agents section. */
export async function openConnect() {
  const status = document.createElement("div");
  status.className = "agent-status";
  const paths = await api.agentPaths().catch(() => []);
  for (const a of paths) {
    const row = document.createElement("div");
    row.className = "agent-row";
    const name = document.createElement("span");
    name.className = "agent-name";
    name.textContent = agentProduct(a.name);
    const where = document.createElement("span");
    where.className = a.found ? "agent-found" : "agent-missing";
    where.textContent = a.found
      ? a.found
      : `not found${a.configured ? ` (configured: ${a.configured})` : ""}; set ${a.name} = "/path/to/${a.name}" under [paths] in .recalc/config.toml`;
    row.append(name, where);
    if (a.found) {
      const caps = document.createElement("span");
      caps.className = "agent-caps";
      caps.textContent = a.reports_usage ? "reports tokens and cost per run" : "plain-text output: runs carry no token or cost figures";
      caps.title = a.reports_usage ? "" : "Add --output-format json (claude) or --json (codex) to the template under [agents] to get them";
      row.appendChild(caps);
    }
    status.appendChild(row);
  }
  if (paths.length === 0) {
    const row = document.createElement("div");
    row.className = "agent-row";
    row.textContent = "Could not check the agents on this machine";
    status.appendChild(row);
  }
  const connect = await api.connectCommand().catch(() => null);
  if (connect) {
    const row = document.createElement("div");
    row.className = "agent-connect";
    const say = document.createElement("div");
    say.textContent = "To connect a Claude Code you run yourself, run this in the board's folder:";
    const code = document.createElement("code");
    code.textContent = connect;
    const copy = document.createElement("button");
    copy.className = "act";
    copy.textContent = "Copy";
    copy.onclick = () => void api.copyText(connect).then(() => (copy.textContent = "Copied"));
    row.append(say, code, copy);
    status.appendChild(row);
  }
  const sheet = cheatsheet();
  queueMicrotask(() =>
    [...sheet.querySelectorAll("h3")].find((h) => h.textContent === "Agents")?.scrollIntoView?.(),
  );
  openOverlay("Connect an agent", status, sheet);
}

/** What the start screen calls each example: a short name and one sentence.
 * An example missing here shows its folder name and its own blurb. */
const EXAMPLES: Record<string, [string, string]> = {
  "done-means-green": ["Done means green", "Claude Code sessions that can't finish until their tests pass."],
  "flaky-hunt": ["Flaky test hunt", "Run suspect tests 50 times, name each cause, and prove every fix."],
  "review-lanes": ["Review lanes", "Score AI code reviewers on open pull requests by how often they're right."],
  "release-gate": ["Release gate", "Checks, a sign-off and release notes that must all pass before you tag."],
  "beads-board": ["Issue board for beads", "Your beads issues as rows, each held to its test before it closes."],
  "claims-ledger": ["Claims ledger", "Four agents in one repo, each claim rechecked as the code changes."],
  "cve-triage": ["Dependency triage", "Scanner advisories checked against the code you actually ship."],
  bench: ["Benchmarks", "Track scores over time and stop on a regression."],
  models: ["Claude and Codex side by side", "Give both the same tasks and grade their answers."],
  "sec-questionnaire": ["Security questionnaire", "Answer a customer's questions from your policies, checked against your Terraform."],
  "soc2-evidence": ["SOC 2 evidence", "Test ten controls over six months of exports with checks an auditor can re-run."],
  legal: ["Legal due diligence", "Six contracts read side by side, with the risky clauses flagged."],
  tax: ["Tax document intake", "Read five tax forms into numbers and reconcile them with the client's summary."],
  practice: ["Tax practice clients", "One question across eight clients, with missing documents and request letters."],
  entities: ["Entity returns", "K-1s from an S corporation, a partnership and a trust checked against a 1040."],
  research: ["Research matrix", "Compare five paper abstracts, then summarize the whole table."],
  scorp: ["S-corp returns", "Five clients' books onto the IRS's draft Form 1120-S, one sheet per client, checked line by line."],
};

/** Which section an example sits in, in order; unlisted ones go under Documents. */
const CODE_EXAMPLES = ["done-means-green", "flaky-hunt", "review-lanes", "release-gate", "beads-board", "claims-ledger", "cve-triage", "bench", "models", "sec-questionnaire", "soc2-evidence"];
const DOCUMENT_EXAMPLES = ["legal", "tax", "practice", "entities", "scorp", "research"];

/** What the start screen and Insert Template call each pattern. */
const PATTERNS: Record<string, [string, string]> = {
  welcome: ["Welcome tour", "Every kind of cell, live, on one small board."],
  review: ["Document grid", "Documents down column A and a question in each column."],
  reviewer: ["Reviewer column", "A second agent checks the first one's answers."],
  verify: ["Verification board", "An agent's claim next to a live test result."],
  mission: ["Mission control", "Watch several agents from one grid, one row per task."],
  batch: ["Batch board", "Run one task across a column of inputs."],
  eval: ["Eval loop", "Edit a prompt, run it and score the results."],
  multiagent: ["Multi-agent locks", "Agents claim files in one repo, and checks catch edits outside them."],
  handoff: ["Draft, review, revise", "One agent drafts, another reviews, and the first revises."],
};

/** A pattern's name and sentence; a board saved with `recalc save` goes by its own name. */
function patternCard(t: { name: string; note: string }): [string, string] {
  if (t.name.startsWith("examples/")) return [t.name.slice(9).replace(/\.recalc(template)?$/, ""), "Saved on this board."];
  return PATTERNS[t.name] ?? [t.name, t.note];
}

/** The first letter as a capital, for notes from elsewhere. */
const sentence = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);

interface StartSection {
  name: string;
  /** A heading in the list above this section. */
  group?: string;
  lead: string;
  /** Said between the lead and the cards. */
  note?: HTMLElement;
  cards: HTMLElement[];
}

/** What the start screen's cards do. */
export interface StartHandlers {
  /** A new board (the open one when it's empty), then `fill` on it. */
  startNew(fill?: () => unknown): void;
  /** Pick a folder of documents for column A, then the first question. */
  documentGrid(): unknown;
}

/** The start screen: sections down the left, the chosen one's cards on the
 * right. Every card makes a new board, or fills the open one when it's empty.
 * Shown as the welcome on an empty board, and from File → New Board…. */
export async function openWelcome(h: StartHandlers, title: string, open = "New") {
  const [demos, templates] = await Promise.all([api.demos().catch((e) => String(e)), api.templates().catch((e) => String(e))]);
  const card = (label: string, note: string, onPick: () => void) => {
    const el = row(label, note, onPick);
    el.classList.add("card");
    return el;
  };
  const unlisted = (what: string, e: string) => row("", `Couldn't list the ${what}. ${sentence(e)}`, () => {}, undefined, true);
  const demoCards = (names: string[] | null) => {
    if (typeof demos === "string") return [unlisted("examples", demos)];
    const known = [...CODE_EXAMPLES, ...DOCUMENT_EXAMPLES];
    const picked = names ? names.flatMap((n) => demos.filter((d) => d.name === n)) : demos.filter((d) => !known.includes(d.name));
    return picked.map((d) => {
      const [label, note] = EXAMPLES[d.name] ?? [d.name, sentence(d.blurb)];
      return card(label, note, () => void api.openDemo(d.name).catch(reportError));
    });
  };
  const sections: StartSection[] = [
    {
      name: "New",
      lead: "Cells hold values, formulas, shell commands and answers from AI agents.",
      note: agentNote(),
      cards: [
        card("Blank board", "Start from an empty grid.", () => h.startNew()),
        card("Document grid from a folder", "Pick a folder. Each document gets a row and each question a column.", () => h.startNew(h.documentGrid)),
        card("Describe it to the Driver", "Say what you need in plain words and the Driver sets up the board.", () => h.startNew(() => showTray("driver"))),
        card("Take a tour", PATTERNS.welcome[1], () => h.startNew(() => api.loadTemplate("welcome"))),
      ],
    },
    {
      name: "Templates",
      lead: "Start a board from one of these patterns.",
      // The tour is a card under New.
      cards:
        typeof templates === "string"
          ? [unlisted("templates", templates)]
          : templates
              .filter((t) => t.name !== "welcome")
              .map((t) => {
                const [label, note] = patternCard(t);
                return card(label, note, () => h.startNew(() => api.loadTemplate(t.name)));
              }),
    },
    { name: "Coding agents", group: "Examples", lead: "Each example opens in a new folder with a sample repo.", cards: demoCards(CODE_EXAMPLES) },
    { name: "Documents", lead: "Each example opens in a new folder with sample files.", cards: [...demoCards(DOCUMENT_EXAMPLES), ...demoCards(null)] },
  ];

  const nav = document.createElement("div");
  nav.className = "start-nav";
  nav.setAttribute("role", "tablist");
  nav.setAttribute("aria-orientation", "vertical");
  const panes = document.createElement("div");
  panes.className = "start-panes";
  const tabs = sections.map((s) => {
    if (s.group) {
      const g = document.createElement("div");
      g.className = "start-group";
      g.textContent = s.group;
      nav.appendChild(g);
    }
    const tab = document.createElement("button");
    tab.className = "start-tab";
    tab.setAttribute("role", "tab");
    const name = document.createElement("span");
    name.textContent = s.name;
    tab.appendChild(name);
    // Counts only where there's a list to browse.
    if (s.name !== "New") {
      const n = document.createElement("span");
      n.className = "count";
      n.textContent = String(s.cards.filter((c) => c.classList.contains("card")).length);
      tab.appendChild(n);
    }
    const pane = document.createElement("div");
    pane.className = "start-pane";
    pane.setAttribute("role", "tabpanel");
    pane.setAttribute("aria-label", s.name);
    const h = document.createElement("h2");
    h.textContent = s.name;
    const lead = document.createElement("p");
    lead.className = "start-lead";
    lead.textContent = s.lead;
    const cards = document.createElement("div");
    cards.className = "start-cards";
    cards.append(...s.cards);
    pane.append(h, lead, ...(s.note ? [s.note] : []), cards);
    panes.appendChild(pane);
    nav.appendChild(tab);
    return { tab, pane };
  });
  const show = (i: number) =>
    tabs.forEach(({ tab, pane }, j) => {
      tab.setAttribute("aria-selected", String(i === j));
      tab.tabIndex = i === j ? 0 : -1;
      pane.hidden = i !== j;
    });
  tabs.forEach(({ tab }, i) => {
    tab.onclick = () => show(i);
    tab.onkeydown = (e) => {
      const to = e.key === "ArrowDown" ? i + 1 : e.key === "ArrowUp" ? i - 1 : -1;
      if (to < 0 || to >= tabs.length) return;
      e.preventDefault();
      show(to);
      tabs[to].tab.focus();
    };
  });
  const first = Math.max(0, sections.findIndex((s) => s.name === open));
  show(first);
  const body = document.createElement("div");
  body.className = "start";
  body.append(nav, panes);
  openOverlay(title, body);
  $("overlay").querySelector(".modal")?.classList.add("start-modal");
}

/** A transient confirmation at the bottom of the window; never for errors. */
/** The open board's name, as Finder shows its folder: no ".recalc". */
export function boardName(dir = board.dir): string {
  return (dir.split("/").filter(Boolean).pop() ?? "").replace(/\.recalc$/, "");
}

/** A path with the home folder as ~. */
export function tildePath(path: string): string {
  return path.replace(/^\/Users\/[^/]+/, "~").replace(/^\/home\/[^/]+/, "~");
}

/** The board's name and place, as clicking a document's title does in
 * Numbers: rename it where it is, or move it. */
export function openBoardInfo() {
  document.getElementById("board-info")?.remove();
  const pop = document.createElement("div");
  pop.id = "board-info";
  pop.className = "dropdown board-info";
  const nameRow = document.createElement("label");
  nameRow.className = "bi-row";
  const nameLabel = document.createElement("span");
  nameLabel.className = "bi-label";
  nameLabel.textContent = "Name";
  const input = document.createElement("input");
  input.className = "bi-name";
  input.value = boardName();
  input.spellcheck = false;
  nameRow.append(nameLabel, input);
  const whereRow = document.createElement("div");
  whereRow.className = "bi-row";
  const whereLabel = document.createElement("span");
  whereLabel.className = "bi-label";
  whereLabel.textContent = "Where";
  const where = document.createElement("span");
  where.className = "bi-where";
  const parent = board.dir.replace(/\/[^/]+\/?$/, "");
  // Cut from the left so the folder's own name shows; the marks keep the
  // path's slashes in order inside right-to-left overflow.
  where.textContent = `\u200e${tildePath(parent)}\u200e`;
  where.title = parent;
  const move = document.createElement("button");
  move.className = "bi-move";
  move.textContent = "Move…";
  move.title = "Move this board's folder somewhere else";
  whereRow.append(whereLabel, where, move);
  const said = document.createElement("div");
  said.className = "bi-said";
  said.hidden = true;
  pop.append(nameRow, whereRow, said);
  const close = () => {
    pop.remove();
    document.removeEventListener("mousedown", outside, true);
  };
  const outside = (e: MouseEvent) => {
    if (!pop.contains(e.target as Node)) close();
  };
  const fail = (e: unknown) => {
    said.textContent = String((e as Error)?.message ?? e);
    said.hidden = false;
  };
  const rename = () => {
    const name = input.value.trim();
    if (!name || name === boardName()) return close();
    api.renameBoard(name).then(close, fail);
  };
  input.onkeydown = (e) => {
    e.stopPropagation();
    if (e.key === "Enter") {
      e.preventDefault();
      rename();
    } else if (e.key === "Escape") close();
  };
  move.onclick = () => {
    api.moveBoard().then((to) => to && close(), fail);
  };
  const anchor = document.getElementById("board-name");
  const r = anchor?.offsetParent ? anchor.getBoundingClientRect() : null;
  document.body.appendChild(pop);
  const w = pop.offsetWidth;
  pop.style.left = `${Math.max(8, (r ? r.left + r.width / 2 : window.innerWidth / 2) - w / 2)}px`;
  pop.style.top = `${r ? r.bottom + 4 : 40}px`;
  document.addEventListener("mousedown", outside, true);
  input.focus();
  input.select();
}

export function toast(text: string) {
  document.getElementById("toast")?.remove();
  const el = document.createElement("div");
  el.id = "toast";
  el.textContent = text;
  document.body.appendChild(el);
  window.setTimeout(() => el.classList.add("show"), 10);
  window.setTimeout(() => el.remove(), 2600);
}

/** A yes/no question in the overlay; resolves false when dismissed. */
/** A message with one button that closes it. */
export function notice(title: string, detail: string) {
  const body = document.createElement("div");
  body.className = "welcome-intro";
  body.textContent = detail;
  const acts = document.createElement("div");
  acts.className = "acts modal-acts";
  const ok = document.createElement("button");
  ok.className = "act primary";
  ok.textContent = "OK";
  ok.onclick = () => closeOverlay();
  acts.appendChild(ok);
  openOverlay(title, body, acts);
}

export function confirm(title: string, detail: string, okLabel: string): Promise<boolean> {
  return new Promise((resolve) => {
    const body = document.createElement("div");
    body.className = "welcome-intro";
    body.textContent = detail;
    const acts = document.createElement("div");
    acts.className = "acts modal-acts";
    let done = false;
    const finish = (v: boolean) => {
      if (done) return;
      done = true;
      closeOverlay();
      resolve(v);
    };
    const ok = document.createElement("button");
    ok.className = "act danger";
    ok.textContent = okLabel;
    ok.onclick = () => finish(true);
    const cancel = document.createElement("button");
    cancel.className = "act";
    cancel.textContent = "Cancel";
    cancel.onclick = () => finish(false);
    acts.append(ok, cancel);
    openOverlay(title, body, acts);
    const watch = new MutationObserver(() => {
      if ($("overlay").hidden) {
        watch.disconnect();
        finish(false);
      }
    });
    watch.observe($("overlay"), { attributes: true });
    queueMicrotask(() => cancel.focus());
  });
}

/** Pick which of a column's values to show: a checklist with counts, all ticked.
 * Resolves the ticked values, or null when dismissed. */
export function pickValues(title: string, values: Array<[string, number]>, ticked: Set<string> | null): Promise<Set<string> | null> {
  return new Promise((resolve) => {
    const list = document.createElement("div");
    list.className = "value-list";
    const boxes: Array<[string, HTMLInputElement]> = [];
    for (const [value, count] of values) {
      const row = document.createElement("label");
      row.className = "value-row";
      const box = document.createElement("input");
      box.type = "checkbox";
      box.checked = ticked ? ticked.has(value) : true;
      const text = document.createElement("span");
      text.className = "value-text";
      text.textContent = value === "" ? "(empty)" : value;
      text.title = value;
      const n = document.createElement("span");
      n.className = "count";
      n.textContent = String(count);
      row.append(box, text, n);
      list.appendChild(row);
      boxes.push([value, box]);
    }
    const acts = document.createElement("div");
    acts.className = "acts modal-acts";
    let done = false;
    const finish = (v: Set<string> | null) => {
      if (done) return;
      done = true;
      closeOverlay();
      resolve(v);
    };
    const all = document.createElement("button");
    all.className = "act";
    all.textContent = "Select all";
    all.onclick = () => boxes.forEach(([, b]) => (b.checked = true));
    const none = document.createElement("button");
    none.className = "act";
    none.textContent = "Select none";
    none.onclick = () => boxes.forEach(([, b]) => (b.checked = false));
    const ok = document.createElement("button");
    ok.className = "act primary";
    ok.textContent = "Show these rows";
    ok.onclick = () => finish(new Set(boxes.filter(([, b]) => b.checked).map(([v]) => v)));
    const cancel = document.createElement("button");
    cancel.className = "act";
    cancel.textContent = "Cancel";
    cancel.onclick = () => finish(null);
    acts.append(ok, cancel, all, none);
    openOverlay(title, list, acts);
    const watch = new MutationObserver(() => {
      if ($("overlay").hidden) {
        watch.disconnect();
        finish(null);
      }
    });
    watch.observe($("overlay"), { attributes: true });
    queueMicrotask(() => ok.focus());
  });
}

/** A one-line text prompt in the overlay; resolves null when dismissed. */
export function promptText(title: string, placeholder: string, initial = ""): Promise<string | null> {
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.className = "field";
    input.placeholder = placeholder;
    input.value = initial;
    input.spellcheck = false;
    let done = false;
    const finish = (v: string | null) => {
      if (done) return;
      done = true;
      closeOverlay();
      resolve(v);
    };
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter") {
        // Whatever takes focus next (an editor) mustn't get this Enter's line break.
        e.preventDefault();
        finish(input.value);
      }
      if (e.key === "Escape") finish(null);
    });
    const hintEl = document.createElement("div");
    hintEl.className = "hint";
    hintEl.style.padding = "6px 12px";
    hintEl.textContent = "Enter to confirm · Esc to cancel";
    openOverlay(title, input, hintEl);
    const watch = new MutationObserver(() => {
      if ($("overlay").hidden) {
        watch.disconnect();
        finish(null);
      }
    });
    watch.observe($("overlay"), { attributes: true });
    queueMicrotask(() => input.focus());
  });
}

// ---- question column: what a document grid is built from ----

const KIND_LABELS: Array<[api.Column["kind"], string, string]> = [
  ["text", "One sentence", "free text"],
  ["yesno", "Yes / No", "centered"],
  ["date", "Date", "YYYY-MM-DD"],
  ["number", "Number", "digits only"],
  ["currency", "Amount", "a plain number, shown as currency"],
  ["choice", "One of a set", "a fixed set of answers"],
  ["quote", "Quote", "the passage, word for word"],
];

/** The answer types, as the picker offers them. */
export const ANSWER_KINDS: Array<[AnswerType["kind"], string, string]> = [
  ["yesno", "Yes / No", "Yes or No; anything else is sent back"],
  ["date", "A date", "YYYY-MM-DD only"],
  ["number", "A number", "digits only, no units"],
  ["currency", "An amount", "a plain number; a currency code only when it isn't USD"],
  ["choice", "One of a set", "exactly one of the options you list"],
  ["quote", "A quote", "the passage itself, word for word, nothing around it"],
  ["json", "JSON", "one JSON object or array; with keys you list, each must be there"],
  ["text", "One sentence", "free text; nothing is checked"],
];

export interface AnswerTypePick {
  kind: AnswerType["kind"] | null;
  options: string[];
  strict: boolean;
}

/** Pick the answer type for the selection. Resolves null when dismissed;
 * kind null means "take any answer". */
export function openAnswerTypePicker(current: AnswerType | null): Promise<AnswerTypePick | null> {
  return new Promise((resolve) => {
    let done = false;
    const finish = (v: AnswerTypePick | null) => {
      if (done) return;
      done = true;
      closeOverlay();
      resolve(v);
    };
    const lenient = document.createElement("label");
    lenient.className = "answer-type-lenient";
    const box = document.createElement("input");
    box.type = "checkbox";
    box.checked = current ? !current.strict : false;
    lenient.append(box, document.createTextNode(" Accept answers that don't fit, but mark them"));
    const list = rowsList();
    list.classList.add("aligned");
    for (const [kind, label, note] of ANSWER_KINDS) {
      const el = row(label, note, () => {}, current?.kind === kind ? "✓ Current" : undefined, true);
      if (current?.kind === kind) el.classList.add("active");
      el.onclick = () => {
        if (kind === "json") {
          const strict = !box.checked;
          closeOverlay();
          done = true;
          void promptText("Which keys must the JSON have?", "top-level keys, comma-separated; empty for any JSON", current?.kind === "json" ? (current.options ?? []).join(", ") : "").then((text) => {
            const options = (text ?? "").split(",").map((o) => o.trim()).filter(Boolean);
            resolve(text === null ? null : { kind: "json", options, strict });
          });
          return;
        }
        if (kind !== "choice") {
          finish({ kind, options: [], strict: !box.checked });
          return;
        }
        const strict = !box.checked;
        closeOverlay();
        done = true;
        void promptText("One of which answers?", "the options, comma-separated", (current?.options ?? []).join(", ")).then((text) => {
          const options = (text ?? "").split(",").map((o) => o.trim()).filter(Boolean);
          resolve(text === null ? null : options.length >= 2 ? { kind: "choice", options, strict } : null);
        });
      };
      list.appendChild(el);
    }
    if (current) list.appendChild(row("Any answer", "no check on what an agent writes here", () => finish({ kind: null, options: [], strict: true })));
    const hintEl = document.createElement("div");
    hintEl.className = "hint";
    hintEl.style.padding = "6px 12px";
    hintEl.textContent = "An agent's answer that doesn't fit is sent back with the type asked for, and the agent answers again. Nothing you type here is checked.";
    openOverlay("Answer type", list, lenient, hintEl);
    const watch = new MutationObserver(() => {
      if ($("overlay").hidden) {
        watch.disconnect();
        finish(null);
      }
    });
    watch.observe($("overlay"), { attributes: true });
  });
}

export interface ColumnSubmit {
  column: api.Column;
  agent: string;
  /** The model its cells run on; empty for the agent's default. */
  model: string;
  /** The column the question asks about, 1 for A. */
  about: number;
  run: boolean;
  save: boolean;
}

/** A column a question can ask about: its number and how the picker names it. */
export interface AboutColumn {
  col: number;
  label: string;
}

/**
 * The Add Question Column dialog: pick a library column or write one (question,
 * answer type, agent, the column it asks about) to add across every row.
 * Resolves null when dismissed.
 */
export function openQuestionColumn(
  library: api.Column[],
  agents: string[],
  onForget: (name: string) => Promise<void>,
  runLabel: (about: number) => string = () => "Run now",
  columns: AboutColumn[] = [{ col: 1, label: "A" }],
  about = 1,
  models: string[] = [],
): Promise<ColumnSubmit | null> {
  return new Promise((resolve) => {
    let done = false;
    const finish = (v: ColumnSubmit | null) => {
      if (done) return;
      done = true;
      closeOverlay();
      resolve(v);
    };
    const form = document.createElement("div");
    form.className = "qcol";

    const question = document.createElement("textarea");
    question.className = "field qcol-question";
    question.placeholder = "The question, as you'd ask a junior reviewer. It becomes the column header.";
    question.rows = 2;
    question.spellcheck = true;

    const nameRow = document.createElement("div");
    nameRow.className = "qcol-row";
    const name = document.createElement("input");
    name.className = "field qcol-name";
    name.placeholder = "Column name (short)";
    name.spellcheck = false;
    const agentSel = document.createElement("select");
    agentSel.className = "field qcol-agent";
    for (const a of agents.length ? agents : ["claude", "codex"]) {
      const o = document.createElement("option");
      o.value = a;
      o.textContent = agentProduct(a);
      agentSel.appendChild(o);
    }
    // The model is free text: an alias from [models], or any model id.
    const modelIn = document.createElement("input");
    modelIn.className = "field qcol-model";
    modelIn.placeholder = "Default model";
    modelIn.spellcheck = false;
    modelIn.title = "The model these cells run on: a name under [models] in the board's settings, or a model id. Empty runs the agent's default.";
    if (models.length) {
      const list = document.createElement("datalist");
      list.id = "qcol-models";
      for (const m of models) {
        const o = document.createElement("option");
        o.value = m;
        list.appendChild(o);
      }
      modelIn.setAttribute("list", list.id);
      nameRow.appendChild(list);
    }
    nameRow.append(name, agentSel, modelIn);

    // Which column each row's question is about.
    const aboutRow = document.createElement("label");
    aboutRow.className = "qcol-about";
    aboutRow.append("Ask about");
    const aboutSel = document.createElement("select");
    aboutSel.className = "field qcol-about-col";
    for (const c of columns) {
      const o = document.createElement("option");
      o.value = String(c.col);
      o.textContent = c.label;
      aboutSel.appendChild(o);
    }
    aboutSel.value = String(about);
    const aboutNote = document.createElement("span");
    aboutNote.className = "hint";
    aboutNote.textContent = "in each row";
    aboutRow.append(aboutSel, aboutNote);

    let kind: api.Column["kind"] = "text";
    const kinds = document.createElement("div");
    kinds.className = "segmented qcol-kinds";
    const options = document.createElement("input");
    options.className = "field qcol-options";
    options.placeholder = "Choices, comma-separated: Consent required, Notice only, Unrestricted";
    options.spellcheck = false;
    options.hidden = true;
    const paintKinds = () => {
      kinds.replaceChildren();
      for (const [id, label, note] of KIND_LABELS) {
        const b = document.createElement("button");
        b.className = "seg" + (kind === id ? " on" : "");
        b.textContent = label;
        b.title = note;
        b.onclick = () => {
          kind = id;
          options.hidden = kind !== "choice";
          paintKinds();
          if (kind === "choice") options.focus();
        };
        kinds.appendChild(b);
      }
    };
    paintKinds();

    const pick = (c: api.Column) => {
      name.value = c.name;
      question.value = c.question;
      kind = c.kind;
      options.value = (c.options ?? []).join(", ");
      options.hidden = kind !== "choice";
      paintKinds();
      question.focus();
    };

    // The library, filtered by the name box.
    const lib = document.createElement("div");
    lib.className = "qcol-lib rows aligned";
    const paintLib = () => {
      lib.replaceChildren();
      const q = name.value.trim().toLowerCase();
      const hits = library.filter((c) => !q || c.name.toLowerCase().includes(q) || c.question.toLowerCase().includes(q));
      for (const c of hits.slice(0, 40)) {
        const r = row(c.name, c.question, () => pick(c), undefined, true);
        r.classList.add("qcol-libitem");
        const tag = document.createElement("span");
        tag.className = "shortcut";
        tag.textContent = KIND_LABELS.find((k) => k[0] === c.kind)?.[1] ?? c.kind;
        r.appendChild(tag);
        if (!c.builtin) {
          const rm = document.createElement("button");
          rm.className = "act small qcol-forget";
          rm.textContent = "Remove";
          rm.title = "Remove from your library";
          rm.onclick = (e) => {
            e.stopPropagation();
            void onForget(c.name).then(() => {
              library = library.filter((x) => x.name !== c.name);
              paintLib();
            });
          };
          r.appendChild(rm);
        }
        lib.appendChild(r);
      }
      if (hits.length === 0) {
        const none = document.createElement("div");
        none.className = "hint";
        none.textContent = "Nothing in the library matches; write the question above.";
        lib.appendChild(none);
      }
    };
    name.addEventListener("input", paintLib);
    paintLib();

    const foot = document.createElement("div");
    foot.className = "acts modal-acts qcol-foot";
    const runBox = checkbox(runLabel(about), true, "Run the new cells as soon as they're added");
    aboutSel.onchange = () => (runBox.el.lastChild!.textContent = runLabel(Number(aboutSel.value)));
    const saveBox = checkbox("Save to my library", false, "keep this column for the next table");
    const add = document.createElement("button");
    add.className = "act primary";
    add.textContent = "Add column";
    add.onclick = () => {
      const qText = question.value.trim();
      if (!qText) {
        question.focus();
        return;
      }
      const column: api.Column = {
        name: name.value.trim() || qText.split(/[?.]/)[0].slice(0, 40),
        question: qText,
        kind,
        options: kind === "choice" ? options.value.split(",").map((o) => o.trim()).filter(Boolean) : [],
      };
      if (kind === "choice" && column.options!.length < 2) {
        options.focus();
        return;
      }
      finish({ column, agent: agentSel.value, model: modelIn.value.trim(), about: Number(aboutSel.value), run: runBox.input.checked, save: saveBox.input.checked });
    };
    const cancel = document.createElement("button");
    cancel.className = "act";
    cancel.textContent = "Cancel";
    cancel.onclick = () => finish(null);
    foot.append(add, cancel, runBox.el, saveBox.el);

    question.addEventListener("keydown", (e) => {
      if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) add.click();
    });
    form.append(question, nameRow, aboutRow, kinds, options, lib);
    openOverlay("Add a question column", form, foot);
    $("overlay").querySelector(".modal")?.classList.add("wide");
    const watch = new MutationObserver(() => {
      if ($("overlay").hidden) {
        watch.disconnect();
        finish(null);
      }
    });
    watch.observe($("overlay"), { attributes: true });
    queueMicrotask(() => question.focus());
  });
}

function checkbox(label: string, on: boolean, title: string): { el: HTMLElement; input: HTMLInputElement } {
  const el = document.createElement("label");
  el.className = "check";
  el.title = title;
  const input = document.createElement("input");
  input.type = "checkbox";
  input.checked = on;
  el.append(input, document.createTextNode(label));
  return { el, input };
}

// ---- format panel: every format option, in the side panel's Format view ----

/**
 * The Format view's contents for the selection. `current` is the active cell's
 * format; `apply` gets one patch per click.
 */
export function formatPanelBody(current: Format, value: string, apply: (patch: Partial<Format> | "clear") => void, cell = "this cell"): HTMLElement {
  const panel = document.createElement("div");
  panel.className = "format-body";

  const section = (title: string, cls = ""): HTMLElement => {
    const s = document.createElement("div");
    s.className = "fp-section" + (cls ? ` ${cls}` : "");
    const h = document.createElement("div");
    h.className = "fp-title";
    h.textContent = title;
    s.appendChild(h);
    panel.appendChild(s);
    return s;
  };
  const seg = (parent: HTMLElement, items: Array<{ label: string; on: boolean; title?: string; pick: () => void }>) => {
    const row = document.createElement("div");
    row.className = "segmented";
    for (const it of items) {
      const b = document.createElement("button");
      b.className = "seg" + (it.on ? " on" : "");
      b.textContent = it.label;
      if (it.title) b.title = it.title;
      b.onclick = it.pick;
      row.appendChild(b);
    }
    parent.appendChild(row);
  };

  const text = section("Text");
  seg(text, [
    { label: "B", on: !!current.bold, title: "Bold (⌘B)", pick: () => apply({ bold: !current.bold }) },
    { label: "I", on: !!current.italic, title: "Italic (⌘I)", pick: () => apply({ italic: !current.italic }) },
  ]);
  seg(text, (["left", "center", "right"] as const).map((a) => ({
    label: a === "left" ? "Left" : a === "center" ? "Center" : "Right",
    on: current.align === a,
    pick: () => apply({ align: current.align === a ? undefined : a }),
  })));
  seg(text, [{ label: "Wrap", on: !!current.wrap, title: "Show long text on several lines; its row grows", pick: () => apply({ wrap: !current.wrap }) }]);

  // Presets preview the selected cell's own value; a preset that leaves it
  // unchanged shows a dash. An empty cell has nothing to preview, so every
  // row shows an example instead, and the heading says which it is.
  // "Show as", not "Number": it holds Date and Plain text too, and changes how a
  // value is shown, never what the cell holds.
  const num = section("Show as", "fp-number");
  const shown = value.trim();
  const aside = document.createElement("span");
  aside.className = "fp-aside";
  aside.textContent = shown ? `Preview of ${cell}` : "Examples";
  num.querySelector(".fp-title")!.appendChild(aside);
  const list = document.createElement("div");
  list.className = "fp-list";
  const row = (label: string, on: boolean, sample: string | null, why: string, pick: () => void) => {
    const r = document.createElement("div");
    r.className = "fp-row" + (on ? " on" : "");
    const s = sampleSpan(sample ?? "—");
    if (sample === null) {
      s.classList.add("fp-none");
      s.title = why;
    }
    r.append(labelSpan(label), s);
    r.onclick = pick;
    list.appendChild(r);
  };
  row("Automatic", !current.num, shown || "1000.12", "", () => apply({ num: undefined }));
  for (const p of NUM_PRESETS) {
    const basis = shown || p.exemplar;
    const sample = formatApplies(basis, p.id) ? formatValue(basis, p.id, { decimals: current.decimals, currency: current.currency }) : null;
    row(p.label, current.num === p.id, sample, `${cell} isn't ${p.id === "date" ? "a date" : "a number"}`, () => apply({ num: p.id }));
  }
  num.appendChild(list);
  // Decimal places, and which currency, for the presets that have them.
  if (current.num === "number" || current.num === "currency" || current.num === "percent") {
    const row = document.createElement("div");
    row.className = "fp-controls";
    const places = current.decimals ?? 2;
    const dec = document.createElement("span");
    dec.className = "fp-stepper";
    const less = document.createElement("button");
    less.className = "seg";
    less.textContent = "−";
    less.title = "Fewer decimal places";
    less.disabled = places <= 0;
    less.onclick = () => apply({ decimals: Math.max(0, places - 1) });
    const more = document.createElement("button");
    more.className = "seg";
    more.textContent = "+";
    more.title = "More decimal places";
    more.disabled = places >= 10;
    more.onclick = () => apply({ decimals: Math.min(10, places + 1) });
    const shown = document.createElement("span");
    shown.className = "fp-count";
    shown.textContent = `${places} decimal${places === 1 ? "" : "s"}`;
    dec.append(less, shown, more);
    row.appendChild(dec);
    if (current.num === "currency") {
      const pick = document.createElement("select");
      pick.className = "fp-select";
      pick.title = "Currency";
      const have = current.currency ?? "USD";
      for (const code of CURRENCIES.includes(have) ? CURRENCIES : [have, ...CURRENCIES]) {
        const o = document.createElement("option");
        o.value = code;
        o.textContent = code;
        o.selected = code === have;
        pick.appendChild(o);
      }
      pick.onchange = () => apply({ currency: pick.value === "USD" ? undefined : pick.value });
      row.appendChild(pick);
    }
    num.appendChild(row);
  }

  const fill = section("Fill", "fp-fill");
  const swatches = document.createElement("div");
  swatches.className = "swatches";
  const none = document.createElement("button");
  none.className = "swatch none" + (!current.fill ? " on" : "");
  none.title = "No fill";
  none.onclick = () => apply({ fill: undefined });
  swatches.appendChild(none);
  for (const f of FILLS) {
    const b = document.createElement("button");
    b.className = `swatch fill-${f.id}` + (current.fill === f.id ? " on" : "");
    b.title = f.label;
    b.onclick = () => apply({ fill: f.id });
    swatches.appendChild(b);
  }
  fill.appendChild(swatches);

  // Highlight: fills that follow the value, the first rule that matches winning.
  const hl = section("Highlight", "fp-highlight");
  const rules: Highlight[] = current.highlight ?? [];
  const setRules = (next: Highlight[]) => apply({ highlight: next.length ? next : undefined });
  const WHENS: Array<[Highlight["when"], string]> = [
    ["is", "is"],
    ["contains", "contains"],
    ["above", "is above"],
    ["below", "is below"],
  ];
  rules.forEach((rule, i) => {
    const row = document.createElement("div");
    row.className = "fp-rule";
    const when = document.createElement("select");
    when.className = "fp-select";
    for (const [id, label] of WHENS) {
      const o = document.createElement("option");
      o.value = id;
      o.textContent = label;
      o.selected = rule.when === id;
      when.appendChild(o);
    }
    when.onchange = () => setRules(rules.map((x, j) => (j === i ? { ...x, when: when.value as Highlight["when"] } : x)));
    const value = document.createElement("input");
    value.className = "fp-input";
    value.value = rule.value;
    value.placeholder = rule.when === "above" || rule.when === "below" ? "number" : "text";
    value.spellcheck = false;
    const commit = () => {
      if (value.value !== rule.value) setRules(rules.map((x, j) => (j === i ? { ...x, value: value.value } : x)));
    };
    value.onchange = commit;
    value.onkeydown = (e) => {
      if (e.key === "Enter") value.blur();
      e.stopPropagation();
    };
    const fillPick = document.createElement("span");
    fillPick.className = "fp-rule-fills";
    for (const f of FILLS) {
      const b = document.createElement("button");
      b.className = `swatch small fill-${f.id}` + (rule.fill === f.id ? " on" : "");
      b.title = f.label;
      b.onclick = () => setRules(rules.map((x, j) => (j === i ? { ...x, fill: f.id } : x)));
      fillPick.appendChild(b);
    }
    const remove = document.createElement("button");
    remove.className = "fp-remove";
    remove.textContent = "×";
    remove.title = "Remove this rule";
    remove.onclick = () => setRules(rules.filter((_, j) => j !== i));
    row.append(when, value, remove, fillPick);
    hl.appendChild(row);
  });
  const add = document.createElement("button");
  add.className = "act small";
  add.textContent = "Add rule";
  add.title = "Fill the cell while its value matches, e.g. \"fail\" in red";
  add.disabled = rules.length >= 8;
  add.onclick = () => setRules([...rules, { when: "contains", value: "", fill: "red" }]);
  hl.appendChild(add);

  const foot = document.createElement("div");
  foot.className = "fp-foot";
  const clear = document.createElement("button");
  clear.className = "act small";
  clear.textContent = "Clear formatting";
  clear.onclick = () => apply("clear");
  foot.append(clear);
  panel.appendChild(foot);
  return panel;
}

function labelSpan(text: string): HTMLElement {
  const s = document.createElement("span");
  s.textContent = text;
  return s;
}
function sampleSpan(text: string): HTMLElement {
  const s = document.createElement("span");
  s.className = "fp-sample";
  s.textContent = text;
  return s;
}

// ---- context menu ----

export interface MenuEntry {
  label: string;
  shortcut?: string;
  disabled?: boolean;
  run: () => void;
}

export function showContextMenu(x: number, y: number, entries: Array<MenuEntry | null>) {
  closeContextMenu();
  const menu = document.createElement("div");
  menu.id = "ctx";
  menu.className = "dropdown";
  for (const e of entries) {
    if (!e) {
      const sep = document.createElement("div");
      sep.className = "sep";
      menu.appendChild(sep);
      continue;
    }
    const item = document.createElement("div");
    item.className = "item" + (e.disabled ? " disabled" : "");
    const label = document.createElement("span");
    label.textContent = e.label;
    item.appendChild(label);
    if (e.shortcut) {
      const s = document.createElement("span");
      s.className = "shortcut";
      s.textContent = e.shortcut;
      item.appendChild(s);
    }
    item.onclick = () => {
      closeContextMenu();
      e.run();
    };
    menu.appendChild(item);
  }
  menu.style.left = `${x}px`;
  menu.style.top = `${y}px`;
  document.body.appendChild(menu);
  const r = menu.getBoundingClientRect();
  if (r.right > window.innerWidth) menu.style.left = `${Math.max(0, x - r.width)}px`;
  if (r.bottom > window.innerHeight) menu.style.top = `${Math.max(0, y - r.height)}px`;
}

export function closeContextMenu() {
  document.getElementById("ctx")?.remove();
}

// ---- toolbar & status bar ----

/** Action id, icon, and the short label the toolbar shows; "|" separates groups.
 * The Driver and Format are the side panel's switch, at the row's right end. */
const TOOLBAR: Array<[string, string, string] | "|" | "spacer"> = [
  ["cell.run", "run", "Run"],
  "|",
  ["cell.verify", "check", "Verify"],
  ["cell.flag", "flag", "Flag"],
  "|",
  ["agent.new", "agent", "Agent cell"],
  ["cell.add_column", "column", "Question"],
  "|",
  ["help.palette", "palette", "Commands"],
];

let actionById: (id: string) => Action | undefined;

/** What the Run button's ▾ offers. */
const RUN_MENU = ["cell.run", "cell.run_selection", "cell.driver_answer"];

export function initToolbar(lookup: (id: string) => Action | undefined) {
  actionById = lookup;
  $("board-name").onclick = () => openBoardInfo();
  const bar = $("toolbar");
  for (const entry of TOOLBAR) {
    if (entry === "|" || entry === "spacer") {
      const sep = document.createElement("span");
      sep.className = entry === "|" ? "sep" : "spacer";
      bar.appendChild(sep);
      continue;
    }
    const [id, glyph, label] = entry;
    const a = actionById(id);
    if (!a) continue;
    const btn = document.createElement("button");
    btn.dataset.action = id;
    if (id === "cell.run") btn.classList.add("primary");
    // The most-used actions keep their labels in a narrow window.
    if (id === "cell.run" || id === "cell.verify" || id === "cell.add_column") btn.classList.add("keep-label");
    if (glyph) btn.appendChild(icon(glyph));
    if (label) {
      const text = document.createElement("span");
      text.className = "tb-label";
      text.textContent = label;
      btn.appendChild(text);
    } else {
      btn.classList.add("icon-only");
    }
    // Accessible name even when the label is hidden in a narrow window.
    btn.setAttribute("aria-label", a.label);
    btn.title = a.shortcut ? `${a.title ?? a.label} (${a.shortcut})` : (a.title ?? a.label);
    btn.onclick = () => a.run();
    if (id !== "cell.run") {
      bar.appendChild(btn);
      continue;
    }
    // Run is a split button: the click runs; ▾ offers the other ways to run.
    const split = document.createElement("span");
    split.className = "split";
    const caret = document.createElement("button");
    caret.className = "split-caret";
    caret.dataset.menu = "run";
    caret.appendChild(icon("chevron"));
    caret.title = "More ways to run: Run Selection, or ask the Driver to answer";
    caret.setAttribute("aria-label", "More ways to run");
    caret.setAttribute("aria-haspopup", "menu");
    caret.onclick = () => {
      const r = caret.getBoundingClientRect();
      showContextMenu(r.left - btn.offsetWidth, r.bottom + 4, RUN_MENU.map((menuId) => {
        const m = actionById(menuId);
        return m ? { label: m.label, shortcut: m.shortcut, disabled: m.enabled ? !m.enabled() : false, run: () => m.run() } : null;
      }));
    };
    split.append(btn, caret);
    bar.appendChild(split);
  }
  // The side panel's switch at the row's end: the same buttons, with icons.
  for (const [id, glyph] of [["switch-agents", "driver"], ["switch-format", "format"]] as const) {
    const btn = $(id);
    if (btn.querySelector("svg")) continue;
    const label = document.createElement("span");
    label.className = "tb-label";
    label.append(...[...btn.childNodes].filter((n) => n.nodeType === 3 /* text */));
    btn.prepend(icon(glyph), label);
  }
}

/** Re-evaluate toolbar enabled/disabled after each render. */
export function updateToolbar() {
  for (const btn of document.querySelectorAll<HTMLButtonElement>("#toolbar button")) {
    const a = actionById(btn.dataset.action!);
    if (a) btn.disabled = a.enabled ? !a.enabled() : false;
  }
}

/** The status bar: location, selection, working agents, and what needs attention. */
export function renderStatus(showDecisions: () => void, stopAll: () => void, filterLabel: string, setBudget: () => void, clearFilter: () => void) {
  // Folder name with its location shortened (temp paths are noise); the full path
  // is in the tooltip.
  const home = board.dir.replace(/^\/Users\/[^/]+/, "~").replace(/^\/home\/[^/]+/, "~");
  const parts = home.split("/").filter(Boolean);
  $("status-dir").textContent = parts.length > 3 ? `…/${parts.slice(-2).join("/")}` : home;
  $("status-dir").title = board.dir;
  // The board's name, as a document app titles its window: no ".recalc".
  $("board-name").textContent = boardName();

  const r = board.selectionRect();
  const names = board.selectedNames();
  const sel = $("status-selection");
  if (r && names.length > 1) {
    sel.textContent = `${rectLabel(r)} · ${names.length} cells`;
  } else {
    sel.textContent = "";
  }

  // One count of everything waiting on a person: questions, suggestions, changed
  // inputs, errors, flags, and sessions that look like they're waiting. Finished
  // answers are news, not counted.
  const aw = $("status-awaiting");
  const n = needsAction().length;
  aw.replaceChildren();
  if (n > 0) {
    aw.appendChild(icon("bell", 13));
    aw.appendChild(document.createTextNode(` ${n} need${n === 1 ? "s" : ""} you`));
  }
  aw.classList.toggle("active", n > 0);
  aw.setAttribute("aria-label", n > 0 ? `${n} need${n === 1 ? "s" : ""} you` : "Nothing needs you");
  aw.title = "Show the questions, suggestions, changed inputs, errors, and flags waiting on you (⌘J)";
  aw.onclick = showDecisions;

  // Board spend, always shown.
  let cost = 0;
  let tokens = 0;
  let priced = 0;
  for (const c of board.everywhere()) {
    if (!c.usage) continue;
    tokens += c.usage.input_tokens + c.usage.output_tokens;
    if (typeof c.usage.cost_usd === "number") {
      cost += c.usage.cost_usd;
      priced++;
    }
  }
  // The Driver counts too, from Claude Code's record of each session it ended.
  const driverCost = board.agents.driver_cost ?? 0;
  const driverTokens = board.agents.driver_tokens ?? 0;
  // A set budget always shows; clicking the spend sets or changes it.
  const sp = $("status-spend");
  sp.replaceChildren();
  const limit = board.agents.budget ?? null;
  const allTokens = tokens + driverTokens;
  if (allTokens > 0 || limit !== null) {
    const bits = allTokens > 0 ? [`${tokensLabel(allTokens)} tokens`] : [];
    if (driverCost > 0 && limit !== null) bits.unshift(`Driver ${costLabel(driverCost)}`);
    if (limit !== null) bits.unshift(`${costLabel(priced > 0 ? cost : (board.agents.spent ?? 0))} of ${costLabel(limit)} budget`);
    else if (priced > 0 || driverCost > 0) bits.unshift(`${costLabel(cost + driverCost)} at API prices`);
    sp.textContent = bits.join(" · ");
    const parts = driverCost > 0 || driverTokens > 0
      ? ` Agent cells: ${costLabel(cost)}, ${tokensLabel(tokens)} tokens. The Driver: ${costLabel(driverCost)}, ${tokensLabel(driverTokens)} tokens, counted as each of its sessions ends.${limit !== null ? " The budget covers agent cells." : ""}`
      : "";
    sp.title = `${priced > 0 || driverCost > 0 ? `What the agents on this board would cost, and the tokens they used.${parts} ${API_PRICES}` : tokens > 0 ? "Tokens the agent runs on this board reported; this agent doesn't price its runs." : "No agent has run on this board yet."} Click to set the agent budget`;
  }
  sp.onclick = setBudget;
  sp.style.cursor = "pointer";
  void working;

  const ag = $("status-agents");
  const { running, queued, max } = board.agents;
  ag.replaceChildren();
  const busy = board.everywhere().filter((c) => {
    const k = c.state;
    return k === "computing" || k === "queued";
  }).length;
  const pausedUntil = board.agents.paused_until ?? null;
  const budget = board.agents.budget ?? null;
  const spent = board.agents.spent ?? 0;
  ag.title = "";
  if (budget !== null && spent >= budget && queued > 0) {
    ag.appendChild(icon("bolt", 13));
    ag.appendChild(document.createTextNode(` Budget reached: ${costLabel(spent)} of ${costLabel(budget)}, ${queued} queued`));
    ag.title = "Queued agent cells wait until you raise the budget (Cell → Set Agent Budget…). A cell you run yourself still runs.";
  } else if (pausedUntil !== null && pausedUntil > Date.now()) {
    const at = new Date(pausedUntil).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
    ag.appendChild(icon("bolt", 13));
    ag.appendChild(document.createTextNode(` Agents held until ${at}: usage limit${queued ? `, ${queued} queued` : ""}`));
    ag.title = "An agent's account hit its usage limit, so queued agent cells wait. Run one yourself to try sooner.";
  } else if (running + queued > 0) {
    ag.appendChild(icon("bolt", 13));
    ag.appendChild(document.createTextNode(` ${running} of ${max} agents running${queued ? `, ${queued} queued` : ""}`));
  } else if (busy > 0) {
    ag.appendChild(icon("bolt", 13));
    ag.appendChild(document.createTextNode(` ${busy} running`));
  }
  const stop = $("status-stop") as HTMLButtonElement;
  stop.hidden = busy === 0;
  stop.onclick = stopAll;
  const filterEl = $("status-filter");
  filterEl.textContent = filterLabel ? `${filterLabel} ✕` : "";
  filterEl.title = filterLabel ? "Show all rows again" : "";
  filterEl.classList.toggle("clickable", filterLabel !== "");
  filterEl.onclick = filterLabel ? clearFilter : null;
}
