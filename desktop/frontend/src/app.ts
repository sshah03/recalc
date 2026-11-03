// Desktop UI wiring: backend snapshots go into the board store, renderers redraw
// what changed, and input events become engine calls. No business logic here.

import { a1Coord, a1Name, colLetters, rectLabel } from "./a1.js";
import * as actions from "./actions.js";
import * as chrome from "./chrome.js";
import * as editor from "./editor.js";
import * as grid from "./grid.js";
import * as inspector from "./inspector.js";
import * as onboard from "./onboard.js";
import * as side from "./side.js";
import { board } from "./state.js";
import * as api from "./tauri.js";
import * as tabs from "./tabs.js";
import { closeTerminal, terminalOpen } from "./terminal.js";
import type { Cell, Format, Snapshot } from "./types.js";
import { costLabel, isAnswer, isDefinition, isRunnable, stateKind } from "./types.js";

const $ = (id: string) => document.getElementById(id)!;

let flash: string | null = null;
// Whether `flash` is a failure (red) or just what a command did.
let flashIsError = false;
/** The last block copied here; pasting it back shifts its references. */
let copied: { origin: string; text: string; block: Array<Array<string | null>>; values: Array<Array<string | null>> } | null = null;
type Filter = "all" | "flagged" | "unverified" | "errors" | "running";
let filter: Filter = "all";
/** Boards whose welcome screen was dismissed this session. */
const welcomed = new Set<string>();

// ---- render ----

function render(changed: Set<string> | null) {
  tabs.renderTabs();
  grid.renderGrid(changed);
  // The named list first: a named cell being edited sits in its row.
  side.renderNamed();
  editor.positionEditor();
  editor.syncFormulaBar();
  side.renderTray();
  renderFormatView();
  chrome.renderStatus(() => side.showTray("decisions"), stopAll, filterLabel(), setBudget, () => setFilter("all"));
  chrome.updateToolbar();
  actions.syncMenu();
  if (!editor.isEditing() && !inspector.inspectorBusy() && inspectorCares(changed)) inspector.renderInspector();
  if (findOpen()) refreshFind(false);
  maybeWelcome();
  freezeDocumentGrid();
}

/** False until this board's layout is read; saving earlier would overwrite it. */
let layoutLoaded = false;

/** Freeze row 1 and column A the first time the board is a document grid.
 * Only once, so Unfreeze Panes sticks. */
function freezeDocumentGrid() {
  if (!layoutLoaded) return;
  const layout = grid.getLayout();
  if (layout.gridFrozen) return;
  const isGrid = [...board.cells.values()].some((c) => {
    if (c.kind !== "agent_call" || !c.source?.includes("{$A")) return false;
    const co = a1Coord(c.name);
    return co !== null && co[0] >= 2 && co[1] >= 2;
  });
  if (!isGrid) return;
  grid.setLayout({ frozenRows: Math.max(1, layout.frozenRows), frozenCols: Math.max(1, layout.frozenCols), gridFrozen: true });
}

/** Does a delta touch what the inspector is showing? */
function inspectorCares(changed: Set<string> | null): boolean {
  if (!changed || changed.size === 0) return true;
  // With nothing selected it counts the agents still to run.
  if (board.selectedNames().length === 0) return true;
  const active = board.active;
  if (!active) return board.selectedNames().some((n) => changed.has(n));
  if (changed.has(active.name)) return true;
  if (active.deps.some((d) => changed.has(d))) return true;
  return board.selectionRect() !== null && board.selectedNames().some((n) => changed.has(n));
}

async function run(p: Promise<unknown>) {
  try {
    flash = null;
    flashIsError = false;
    await p;
  } catch (e) {
    oops(e);
  }
}

function oops(e: unknown) {
  flash = String(e);
  flashIsError = true;
  inspector.renderInspector();
}

function select(name: string, end: string | null = null) {
  if (terminalOpen()) void closeTerminal();
  if (editor.isEditing()) editor.cancel();
  flash = null;
  flashIsError = false;
  grid.closeCard();
  // A cell on another sheet: show that sheet first.
  name = board.reveal(name);
  if (end) end = board.local(board.full(end)).replace(/^.*!/, "");
  board.select(name, end);
  side.followSelection();
  if (a1Coord(name)) grid.scrollIntoView(end ?? name);
  $("grid-wrap").focus({ preventScroll: true });
}

/** The welcome sheet for an empty board, once per board; it closes when cells
 * arrive from elsewhere (e.g. the driver's first table). */
let autoWelcome = false;
function maybeWelcome() {
  if (board.all.size > 0) {
    if (autoWelcome && document.querySelector("#overlay .modal[aria-label='Welcome to recalc']")) chrome.closeOverlay();
    autoWelcome = false;
    return;
  }
  if (!board.dir || welcomed.has(board.dir)) return;
  // A board a start-screen card just made opens without a welcome of its own.
  if (madeByCard) {
    welcomed.add(board.dir);
    madeByCard();
    madeByCard = null;
    return;
  }
  if (chrome.overlayOpen()) return;
  welcomed.add(board.dir);
  autoWelcome = true;
  openWelcome("Welcome to recalc");
}

/** Set while a start-screen card waits for the board it made to arrive. */
let madeByCard: (() => void) | null = null;

/** Make a new board for a start-screen card, then fill it. An empty board is
 * used as it is. */
async function startNew(fill?: () => unknown) {
  if (board.all.size > 0) {
    const arrived = new Promise<void>((resolve) => (madeByCard = resolve));
    const dir = await api.newBoard().catch((e) => {
      oops(e);
      return null;
    });
    if (!dir) {
      madeByCard = null;
      return;
    }
    const rename = document.body.classList.contains("mac") ? "Click its name at the top to rename or move it." : "File → Rename… names it.";
    chrome.toast(`New board “${chrome.boardName(dir)}” in ${chrome.tildePath(dir.replace(/\/[^/]+\/?$/, ""))}. ${rename}`);
    // The fill reads this window's copy of the board, so wait for the new one.
    await arrived;
  }
  await Promise.resolve(fill?.()).catch(oops);
}

function openWelcome(title: string, open?: string) {
  void chrome.openWelcome({ startNew: (fill) => void startNew(fill), documentGrid: newReviewTable }, title, open);
}


// ---- edits ----

function commitEdit(name: string, text: string, move: editor.Move) {
  const cell = board.get(name);
  const looksLikeDef = isDefinition(text);
  const computed = cell?.source != null;
  const unchanged = cell ? (cell.source ?? cell.value) === text : text === "";
  // A plain value over a computed cell asks suggest / pin / override instead
  // of dropping the definition.
  const p: Promise<string | null | void> = unchanged
    ? Promise.resolve()
    : computed && !looksLikeDef
      ? api.editComputed(name, text)
      : api.setCell(name, text);
  // Move before the write returns so keys typed meanwhile reach the next cell.
  const co = a1Coord(name);
  if (!co) select(name);
  else if (move === "down") select(a1Name(co[0], co[1] + 1));
  else if (move === "right") select(a1Name(co[0] + 1, co[1]));
  else if (move === "stay") select(name);
  void run(
    p.then((wants) => {
      // A value that doesn't fit the cell's answer type is kept; say what it takes.
      if (!wants) return;
      flash = wants;
      inspector.renderInspector();
    }),
  );
}

function editSelected(seed?: string) {
  const name = board.selection.active;
  if (!name) {
    newCell();
    return;
  }
  editor.beginEdit(name, seed);
}

function newCell() {
  void chrome.promptText("New named cell", "name (letters, digits, _), e.g. status").then((name) => {
    const nm = name?.trim();
    if (!nm) return;
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(nm)) {
      oops(`invalid cell name ${JSON.stringify(nm)}: use letters, digits and _ only`);
      return;
    }
    // Editing starts now, so keys typed straight after the name aren't lost:
    // in the formula bar until the cell's row appears, then in the row.
    select(nm);
    editor.beginEdit(nm, "");
    void run(api.setCell(nm, ""));
  });
}

function deleteSelected() {
  const names = board.selectedNames();
  if (names.length === 0) return;
  const single = names.length === 1 ? board.get(names[0]) : undefined;
  // Clearing an agent-computed cell goes through the computed-cell edit path, not delete.
  if (single && single.source != null && single.def_by?.startsWith("agent:")) {
    void run(api.editComputed(single.name, ""));
    return;
  }
  void run(
    api.deleteCells(names).then((kept) => {
      if (kept.length) {
        flash = `${kept.length} agent-written cell${kept.length === 1 ? "" : "s"} kept (${kept.slice(0, 4).join(", ")}${kept.length > 4 ? ", …" : ""}); delete them one at a time`;
        inspector.renderInspector();
      }
    }),
  );
}

function fillSelection(dir: "down" | "right") {
  const r = board.selectionRect();
  if (!r) return;
  void run(api.fill(a1Name(r[0], r[1]), a1Name(r[2], r[3]), dir));
}

function runWithConfirm(cell: Cell) {
  void api.needsConfirm(cell.name).then((needs) => {
    if (needs) {
      inspector.renderInspector();
      inspector.confirmRun(cell);
    } else {
      void run(api.trigger(cell.name));
    }
  });
}

function runSelected() {
  const cell = board.active;
  if (cell && isRunnable(cell.kind)) runWithConfirm(cell);
}

function runSelection() {
  return runNames(board.selectedNames());
}

/** The board's ! cells, as the palette lists them: running one is choosing it
 * and pressing Run, with Run's confirmations. */
function boardScripts() {
  return board
    .everywhere()
    .filter((c) => c.kind === "action")
    .sort((a, b) => a.name.localeCompare(b.name))
    .map((c) => ({
      label: `Run ${c.name}`,
      note: (c.source ?? "").replace(/^!\s*/, "").split(" # ")[0],
      run: () => {
        select(board.local(c.name));
        runSelected();
      },
    }));
}

/** Above this many cells, asking the Driver checks first: it answers one at a
 * time, and on a big table its context fills. */
const CONFIRM_DRIVER = 20;

/** Ask the Driver, in its terminal, to answer the selected agent cells itself. */
async function driverAnswer() {
  const names = board.selectedNames().filter((n) => board.get(n)?.kind === "agent_call");
  if (!names.length) return;
  if (!side.driverRunning()) {
    side.showTray("driver");
    flash = "Start the Driver, then ask it to answer";
    flashIsError = false;
    inspector.renderInspector();
    return;
  }
  if (
    names.length > CONFIRM_DRIVER &&
    !(await chrome.confirm(
      `Ask the Driver to answer ${names.length} cells?`,
      "It answers them one at a time in its own conversation, and on a table this big its context fills up. Run Selection runs them in parallel, each agent on its own.",
      "Ask the Driver",
    ))
  )
    return;
  if (!(await side.ensureDriverShown())) {
    flash = "Start the Driver, then ask it to answer";
    flashIsError = false;
    inspector.renderInspector();
    return;
  }
  await run(api.driverAnswer(names)).then(() => {
    flash = `Asked the Driver to answer ${names.length === 1 ? names[0] : `${names.length} cells`}`;
    flashIsError = false;
    inspector.renderInspector();
  });
}

// The selection whose last Run found nothing stale: Run again on it forces a re-run.
let upToDateKey = "";

/** Runs above this count or estimated cost ask for confirmation first. */
const CONFIRM_RUNS = 20;
const CONFIRM_USD = 2;

async function runNames(names: string[], confirmed = false) {
  const key = names.join(",");
  const again = key === upToDateKey;
  upToDateKey = "";
  if (!confirmed) {
    // A re-run takes every agent cell; otherwise only those without a current answer.
    const agents = names.filter((n) => {
      const c = board.get(n);
      return c?.kind === "agent_call" && (again || stateKind(c.state) !== "ok");
    });
    const avg = averageCost();
    const estimate = avg === null ? null : avg * agents.length;
    if (agents.length > CONFIRM_RUNS || (estimate !== null && estimate > CONFIRM_USD)) {
      const cost =
        estimate === null
          ? "Nothing on this board has a price yet, so there's no estimate."
          : `That's about ${costLabel(estimate)} at this board's average of ${costLabel(avg!)} a run.`;
      const ok = await chrome.confirm(
        `Run ${agents.length} agent cells?`,
        `${cost} They queue under the board's agent limit, and Stop All Runs (⇧⌘.) ends them.`,
        `Run ${agents.length}`,
      );
      if (!ok) {
        if (again) upToDateKey = key;
        flash = "Nothing was run";
        flashIsError = false;
        inspector.renderInspector();
        return;
      }
    }
  }
  return run(
    api.runSelection(names, again).then(async (out) => {
      out.up_to_date ??= [];
      if (!again && !out.ran.length && out.up_to_date.length && !out.need_confirm.length) {
        upToDateKey = key;
        flash = `All ${out.up_to_date.length} already up to date. Run Selection again to re-run them.`;
        inspector.renderInspector();
        return;
      }
      const bits = [`${out.ran.length} ${again ? "re-run" : "started"}`];
      if (!again && out.up_to_date.length) {
        bits.push(`${out.up_to_date.length} already up to date (Run Selection again to re-run them)`);
      }
      const estimate = estimateCost(out.ran);
      if (estimate) bits.push(`about ${costLabel(estimate)} at this board's average`);
      if (out.need_confirm.length) {
        bits.push(`${out.need_confirm.length} need confirmation (${out.need_confirm.slice(0, 4).join(", ")}); run them one at a time`);
      }
      flash = bits.join(" · ");
      inspector.renderInspector();
    }),
  );
}

/** Estimated cost of running `names`: agent cells times the average cost per run.
 * Null until something has been priced. */
function estimateCost(names: string[]): number | null {
  const avg = averageCost();
  if (avg === null) return null;
  const agents = names.filter((n) => board.get(n)?.kind === "agent_call").length;
  return agents ? avg * agents : null;
}

/** Average reported cost per agent run; null until three runs are priced. */
function averageCost(): number | null {
  const priced = board.everywhere().filter((c) => c.kind === "agent_call" && c.usage?.cost_usd != null);
  if (priced.length < 3) return null;
  return priced.reduce((s, c) => s + (c.usage?.cost_usd ?? 0), 0) / priced.length;
}

function stopSelected() {
  for (const name of board.selectedNames()) {
    const s = board.get(name)?.state;
    if (s === "computing" || s === "queued") void run(api.stop(name));
  }
}

function stopAll() {
  void run(api.stopAll());
}

function setBudget() {
  const { budget, spent = 0 } = board.agents;
  const title = `Agent budget for this board, at API prices: ${costLabel(spent)} so far${budget != null ? `, limit ${costLabel(budget)}` : ", no limit set"}. On a plan, it caps how much of your usage the board's runs take`;
  void chrome.promptText(title, "dollars, e.g. 20; empty for no limit", budget != null ? String(budget) : "").then((text) => {
    if (text === null) return;
    const t = text.trim().replace(/^\$/, "");
    if (t === "") {
      void run(api.setBudget(null));
      return;
    }
    const dollars = Number(t);
    if (!Number.isFinite(dollars) || dollars < 0) {
      oops(`a budget is a number of dollars, like 20; got ${JSON.stringify(text)}`);
      return;
    }
    void run(api.setBudget(dollars));
  });
}

function setMark(kind: "verified" | "flagged" | null, note?: string) {
  const names = board.selectedNames();
  if (names.length) void run(api.setMark(names, kind, note));
}

function toggleVerified() {
  const names = board.selectedNames();
  const all = names.length > 0 && names.every((n) => board.get(n)?.mark?.kind === "verified");
  if (all) return setMark(null);
  // Only cells with an answer can be signed off.
  const signable = names.filter((n) => (board.get(n)?.value ?? "") !== "");
  if (signable.length) void run(api.setMark(signable, "verified"));
}

/** Unsigned answers: grid cells in reading order, then named cells by name. */
function unverifiedAnswers(): string[] {
  const open = [...board.cells.values()].filter((c) => isAnswer(c) && c.mark?.kind !== "verified");
  const grid = open
    .map((c) => ({ name: c.name, co: a1Coord(c.name) }))
    .filter((x): x is { name: string; co: [number, number] } => x.co !== null)
    .sort((a, b) => a.co[1] - b.co[1] || a.co[0] - b.co[0])
    .map((x) => x.name);
  const named = open.filter((c) => a1Coord(c.name) === null).map((c) => c.name).sort();
  return [...grid, ...named];
}

/** Select the next unsigned answer after the selection, wrapping to the top. */
function nextUnverified() {
  const names = unverifiedAnswers();
  if (names.length === 0) {
    flash = "Every answer is verified";
    inspector.renderInspector();
    return;
  }
  const co = board.selection.active ? a1Coord(board.selection.active) : null;
  const after = co
    ? names.find((n) => {
        const c = a1Coord(n)!;
        return c[1] > co[1] || (c[1] === co[1] && c[0] > co[0]);
      })
    : undefined;
  const next = after ?? names[0];
  select(next);
  grid.scrollIntoView(next);
}

/** Sign off on the selection and move to the next answer to check. */
function verifyAndNext() {
  const names = board.selectedNames();
  if (names.length === 0) return;
  void run(api.setMark(names, "verified")).then(() => nextUnverified());
}

// ---- review mode: one answer at a time, from the keyboard ----

/** Review unsigned answers column by column, so one question is read across every
 * document. V signs off, F flags, J skips, K goes back, Esc stops; skipped answers
 * stay unverified. */
interface Review {
  queue: string[];
  at: number;
  verified: number;
  flagged: number;
  skipped: number;
}
let review: Review | null = null;

function startReview() {
  const r = board.selectionRect();
  const inside = (n: string) => {
    if (!r || board.selectedNames().length < 2) return true;
    const co = a1Coord(n)!;
    return co[0] >= r[0] && co[0] <= r[2] && co[1] >= r[1] && co[1] <= r[3];
  };
  const queue = unverifiedAnswers()
    .filter(inside)
    .sort((a, b) => {
      const x = a1Coord(a)!;
      const y = a1Coord(b)!;
      return x[0] - y[0] || x[1] - y[1];
    });
  if (!queue.length) {
    flash = r && board.selectedNames().length > 1 ? "Every answer in the selection is verified" : "Every answer is verified";
    flashIsError = false;
    inspector.renderInspector();
    return;
  }
  review = { queue, at: 0, verified: 0, flagged: 0, skipped: 0 };
  // Show the source passage the answer rests on.
  if (queue.some((n) => board.get(n)?.cite)) side.showTray("source");
  showReviewCell();
}

function showReviewCell() {
  if (!review) return;
  // May have been signed off elsewhere in the meantime.
  while (review.at < review.queue.length && board.get(review.queue[review.at])?.mark?.kind === "verified") review.at++;
  if (review.at >= review.queue.length) return endReview(true);
  const name = review.queue[review.at];
  select(name);
  grid.scrollIntoView(name);
}

function endReview(finished = false) {
  if (!review) return;
  const { verified, flagged, skipped } = review;
  review = null;
  flash = `${finished ? "Review done" : "Review stopped"}: ${verified} verified, ${flagged} flagged, ${skipped} skipped`;
  flashIsError = false;
  inspector.renderInspector();
}

/** The keys a review takes; true when the key was one of them. */
function reviewKey(e: KeyboardEvent): boolean {
  if (!review || e.altKey) return false;
  const name = review.queue[review.at];
  const key = e.key.toLowerCase();
  if (key === "v") {
    void run(api.setMark([name], "verified")).then(() => {
      if (!review) return;
      review.verified++;
      review.at++;
      showReviewCell();
    });
  } else if (key === "f") {
    void chrome.promptText(`Flag ${name}`, "why it needs a look (optional)", board.get(name)?.mark?.note ?? "").then((note) => {
      if (note === null || !review) return;
      void run(api.setMark([name], "flagged", note)).then(() => {
        if (!review) return;
        review.flagged++;
        review.at++;
        showReviewCell();
      });
    });
  } else if (key === "j" || key === "s") {
    review.skipped++;
    review.at++;
    showReviewCell();
  } else if (key === "k") {
    review.at = Math.max(0, review.at - 1);
    select(review.queue[review.at]);
    grid.scrollIntoView(review.queue[review.at]);
  } else if (e.key === "Escape") {
    endReview();
  } else return false;
  e.preventDefault();
  return true;
}

function reviewStatus() {
  if (!review) return null;
  return { at: Math.min(review.at + 1, review.queue.length), total: review.queue.length, name: review.queue[review.at] };
}

function flagSelected() {
  const r = board.selectionRect();
  const first = board.get(board.selectedNames()[0] ?? "");
  const what = r ? rectLabel(r) : (first?.name ?? "");
  void chrome.promptText(`Flag ${what}`, "why it needs a look (optional)", first?.mark?.note ?? "").then((note) => {
    if (note !== null) setMark("flagged", note);
  });
}

// ---- structure: rows and columns ----

function restructure(axis: "rows" | "cols", insert: boolean, after: boolean) {
  const r = board.selectionRect();
  if (!r) return;
  const [c0, r0, c1, r1] = r;
  const start = axis === "rows" ? r0 : c0;
  const end = axis === "rows" ? r1 : c1;
  const n = end - start + 1;
  const at = insert && after ? end + 1 : start;
  if (insert) {
    void run(api.restructure(axis, at, n, true));
    return;
  }
  // Deleting shifts everything after it, so name what goes.
  const filled = board.selectedNames().length === 0 ? 0 : [...board.cells.keys()].filter((name) => {
    const co = a1Coord(name);
    if (!co) return false;
    const i = axis === "rows" ? co[1] : co[0];
    return i >= start && i <= end;
  }).length;
  const what = axis === "rows" ? (n === 1 ? `row ${start}` : `rows ${start}–${end}`) : (n === 1 ? `column ${a1Name(start, 1).replace(/1$/, "")}` : `columns ${a1Name(start, 1).replace(/1$/, "")}–${a1Name(end, 1).replace(/1$/, "")}`);
  void chrome
    .confirm(`Delete ${what}?`, `${filled} cell${filled === 1 ? "" : "s"} will be removed and everything after moves up. References follow their cells, and Undo (⌘Z) puts it all back.`, "Delete")
    .then((yes) => {
      if (yes) void run(api.restructure(axis, at, n, false)).then(() => board.select(a1Name(c0, r0)));
    });
}

// ---- sort and format ----

/** Sort by the active column: the selected rows when several are selected,
 * otherwise the data rows (2 through the last filled row). Row 1 stays put. */
function sortByActiveColumn(descending: boolean) {
  const co = board.selection.active ? a1Coord(board.selection.active) : null;
  if (!co) return;
  const r = board.selectionRect();
  const [first, last] = r && r[3] > r[1] ? [Math.max(2, r[1]), r[3]] : [2, board.extent()[1]];
  if (last <= first) return;
  void run(api.sortRows(co[0], first, last, descending)).then(() => board.select(a1Name(co[0], first)));
}

/** The side panel's Format view: the selection's formatting. Redrawn only when
 * the selection or its format changes, so a click doesn't lose its place. */
let formatKey = "";
function renderFormatView(force = false) {
  if (side.sideShowing() !== "format") return;
  const a = board.active;
  const names = board.selectedNames();
  const key = JSON.stringify([names.length, a?.name, a?.format, a?.value]);
  if (!force && key === formatKey) return;
  formatKey = key;
  const body = $("format-body");
  if (!a && names.length === 0) {
    const empty = document.createElement("div");
    empty.className = "fp-empty";
    empty.textContent = "Select cells to format them.";
    body.replaceChildren(empty);
    return;
  }
  body.replaceChildren(chrome.formatPanelBody(a?.format ?? {}, a?.value ?? "", (patch) => format(patch), a?.name));
}

/** Merge a format patch into each selected cell; toggles flip relative to the active cell. */
function format(what: Partial<Format> | "clear" | "header" | "fill" | "more" | "number" | "bold" | "italic") {
  const names = board.selectedNames();
  const active = board.active?.format ?? {};
  if (what === "more" || what === "fill" || what === "number") {
    side.showSide("format");
    const part = what === "number" ? ".fp-number" : what === "fill" ? ".fp-fill" : null;
    if (part) document.querySelector(`#format-body ${part}`)?.scrollIntoView({ block: "nearest" });
    return;
  }
  if (what === "header") {
    const [lastCol] = board.extent();
    const row1 = Array.from({ length: Math.max(1, lastCol) }, (_, i) => a1Name(i + 1, 1));
    void run(Promise.all(row1.map((n) => api.setFormat([n], { ...(board.get(n)?.format ?? {}), bold: true })))).then(() =>
      grid.setLayout({ frozenRows: Math.max(1, grid.getLayout().frozenRows) }),
    );
    return;
  }
  if (names.length === 0) return;
  if (what === "clear") {
    void run(api.setFormat(names, null));
    return;
  }
  const patch: Partial<Format> = what === "bold" ? { bold: !active.bold } : what === "italic" ? { italic: !active.italic } : what;
  void run(
    Promise.all(
      names.map((n) => {
        const merged: Format = { ...(board.get(n)?.format ?? {}), ...patch };
        for (const k of Object.keys(merged) as Array<keyof Format>) {
          const v = merged[k];
          if (v === undefined || v === false || (Array.isArray(v) && v.length === 0)) delete merged[k];
        }
        return api.setFormat([n], Object.keys(merged).length ? merged : null);
      }),
    ),
  );
}

// ---- document grids: documents down A, a question per column ----

/** Import a folder as column A under a header, then ask for the first question column.
 * Requires an empty A1 (one table per board). */
async function newReviewTable() {
  if (board.get("A1")) {
    flash = "A1 is taken; a document grid starts at A1. Start a new board (⌘N) for another one.";
    flashIsError = true;
    inspector.renderInspector();
    return;
  }
  const n = await api.importFiles("A2").catch((e) => {
    oops(e);
    return 0;
  });
  if (n === 0) return;
  await run(api.setCell("A1", "Document"));
  format("header");
  select("A1");
  await addQuestionColumn();
}

async function addQuestionColumn() {
  const [library, paths, models] = await Promise.all([
    api.columnLibrary().catch(() => [] as api.Column[]),
    api.agentPaths().catch(() => [] as api.AgentPath[]),
    api.modelAliases().catch(() => [] as string[]),
  ]);
  const agents = paths.map((p) => p.name);
  // The columns a question can be about: each with values below its header.
  const [maxCol, maxRow] = board.extent();
  const filled = (col: number) => {
    let n = 0;
    for (let row = 2; row <= maxRow; row++) if (board.get(a1Name(col, row))?.value.trim()) n++;
    return n;
  };
  const isFile = (v: string) => v.split(",").every((p) => /\.[A-Za-z0-9]{1,5}$/.test(p.trim()));
  const columns: chrome.AboutColumn[] = [];
  let about = 1;
  for (let col = 1; col <= maxCol; col++) {
    const n = filled(col);
    if (!n && col !== 1) continue;
    const head = board.get(a1Name(col, 1))?.value.trim();
    columns.push({ col, label: head ? `${colLetters(col)} · ${head.length > 40 ? `${head.slice(0, 39)}…` : head}` : colLetters(col) });
  }
  // Documents, when a column holds them; otherwise column A.
  const docCol = columns.find((c) => {
    const vals: string[] = [];
    for (let row = 2; row <= maxRow; row++) {
      const v = board.get(a1Name(c.col, row))?.value.trim();
      if (v) vals.push(v);
    }
    return vals.length > 0 && vals.filter(isFile).length * 2 >= vals.length;
  });
  if (docCol) about = docCol.col;
  // Label Run now with its run count (one per row with a value to ask about) and estimated cost.
  const each = averageCost();
  const runLabel = (col: number) => {
    const n = filled(col);
    return n ? `Run now: ${n} agent run${n === 1 ? "" : "s"}${each !== null ? `, about ${costLabel(each * n)} at API prices` : ""}` : "Run now";
  };
  const picked = await chrome.openQuestionColumn(library, agents, (name) => api.forgetColumn(name), runLabel, columns, about, models);
  if (!picked) return;
  if (picked.save) await api.saveColumn(picked.column).catch(oops);
  const added = await api.addQuestionColumn(picked.column, picked.agent, picked.about, undefined, picked.model).catch((e) => {
    oops(e);
    return null;
  });
  if (!added) return;
  // The backend names cells in full; the grid by its own names.
  added.header = board.local(added.header);
  added.answers = added.answers.map((n) => board.local(n));
  if (added.answers.length === 0) {
    flash = `${added.header} added; column ${colLetters(picked.about)} has nothing below its header to ask about. Fill it (File → Import Files as Rows puts documents there) and add the column again`;
    select(added.header);
    inspector.renderInspector();
    return;
  }
  select(added.answers[0], added.answers[added.answers.length - 1]);
  // The dialog already showed the count and cost; no second confirmation.
  if (picked.run) void runNames(added.answers, true);
}

/** The answer type agents in the selection must return. */
async function answerTypeSelection() {
  const names = board.selectedNames();
  if (names.length === 0) return;
  // The first selected cell with a type; a header's type applies to the cells below.
  const current = names.map((n) => board.get(n)?.answer_type).find((s) => s) ?? null;
  const pick = await chrome.openAnswerTypePicker(current);
  if (!pick) return;
  await run(api.setAnswerType(names, pick.kind, pick.options, pick.strict));
}

/** Every runnable cell in the active column below row 1. */
function runColumn() {
  const r = board.selectionRect();
  if (!r) return;
  const [, maxRow] = board.extent();
  const names: string[] = [];
  for (let col = r[0]; col <= r[2]; col++) {
    for (let row = 2; row <= maxRow; row++) {
      const c = board.get(a1Name(col, row));
      if (c && isRunnable(c.kind)) names.push(c.name);
    }
  }
  if (names.length) void runNames(names);
}

function toggleSource() {
  if (side.trayShowing() === "source") side.showTray("activity");
  else side.showSource(board.active);
}

// ---- view: wrap, freeze, filter ----

function toggleWrap() {
  grid.setLayout({ wrap: !grid.getLayout().wrap });
}

function toggleNotify() {
  const on = grid.getLayout().notify === false;
  grid.setLayout({ notify: on });
  oops(on ? "Notifications on: recalc tells you when something needs you while it's in the background." : "Notifications off.");
}

function freeze() {
  const co = board.selection.active ? a1Coord(board.selection.active) : null;
  if (!co) return;
  grid.setLayout({ frozenRows: co[1] - 1, frozenCols: co[0] - 1 });
}

function unfreeze() {
  grid.setLayout({ frozenRows: 0, frozenCols: 0 });
}

function rowCells(row: number): Cell[] {
  const out: Cell[] = [];
  for (const c of board.cells.values()) {
    const co = a1Coord(c.name);
    if (co && co[1] === row) out.push(c);
  }
  return out;
}

// Column filters: allowed values per column, compared trimmed and case-insensitively.
// They combine with each other and the preset; row 1 is the header and always shows.
let valueFilters = new Map<number, Set<string>>();
const filterKey = (v: string) => v.trim().toLowerCase();

function valueAt(col: number, row: number): string {
  return board.get(a1Name(col, row))?.value ?? "";
}

function setFilter(which: Filter) {
  filter = which;
  if (which === "all") valueFilters = new Map();
  applyFilters();
}

function filterToValue() {
  const co = board.active ? a1Coord(board.active.name) : null;
  if (!co) return;
  valueFilters.set(co[0], new Set([filterKey(board.active?.value ?? "")]));
  applyFilters();
}

async function filterColumn() {
  const r = board.selectionRect();
  const co = r ? [r[0], r[1]] : board.active ? a1Coord(board.active.name) : null;
  if (!co) return;
  const col = co[0];
  const [, maxR] = board.extent();
  const counts = new Map<string, [string, number]>();
  for (let row = 2; row <= maxR; row++) {
    const v = valueAt(col, row);
    const k = filterKey(v);
    const seen = counts.get(k);
    counts.set(k, [seen?.[0] ?? v.trim(), (seen?.[1] ?? 0) + 1]);
  }
  const values = [...counts.values()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
  const header = valueAt(col, 1) || colLetters(col);
  const current = valueFilters.get(col);
  const ticked = await chrome.pickValues(`Filter ${colLetters(col)}: ${header}`, values, current ? new Set(values.filter(([v]) => current.has(filterKey(v))).map(([v]) => v)) : null);
  if (!ticked) return;
  if (ticked.size === values.length) valueFilters.delete(col);
  else valueFilters.set(col, new Set([...ticked].map(filterKey)));
  applyFilters();
}

function applyFilters() {
  const tests: Record<Filter, ((c: Cell) => boolean) | null> = {
    all: null,
    flagged: (c) => c.mark?.kind === "flagged",
    unverified: (c) => isAnswer(c) && c.mark?.kind !== "verified",
    errors: (c) => stateKind(c.state) === "error",
    running: (c) => stateKind(c.state) === "computing" || stateKind(c.state) === "queued",
  };
  const test = tests[filter];
  const byValue = [...valueFilters.entries()];
  const passes = (row: number) =>
    (!test || rowCells(row).some(test)) && (row === 1 || byValue.every(([col, ok]) => ok.has(filterKey(valueAt(col, row)))));
  grid.setRowFilter(test || byValue.length ? passes : null);
  render(new Set());
}

function filterLabel(): string {
  const parts: string[] = filter === "all" ? [] : [`${filter} rows`];
  for (const [col, ok] of valueFilters) {
    const shown = [...ok][0] ?? "";
    // Use the column letter: headers are often long questions.
    const letter = colLetters(col);
    parts.push(ok.size === 1 ? `${letter} = ${shown === "" ? "(empty)" : valueOriginal(col, shown)}` : `${letter}: ${ok.size} of its values`);
  }
  return parts.length ? `Showing ${parts.join(", ")}` : "";
}

/** A filter key as the column writes it (keys are lowercased). */
function valueOriginal(col: number, key: string): string {
  const [, maxR] = board.extent();
  for (let row = 2; row <= maxR; row++) if (filterKey(valueAt(col, row)) === key) return valueAt(col, row).trim();
  return key;
}

// ---- layout persistence ----

/** The board's saved layout: the first sheet's at the top, the others' under
 * `sheets.<name>` (as the Excel export reads it). */
type LayoutDoc = Partial<grid.Layout> & { sheets?: Record<string, Partial<grid.Layout>> };
let layoutDoc: LayoutDoc = {};

/** The shown sheet's part of the layout. */
function sheetLayout(): Partial<grid.Layout> {
  return board.sheet === null ? layoutDoc : (layoutDoc.sheets?.[board.sheet] ?? {});
}

function saveLayoutDoc() {
  window.clearTimeout(saveTimer);
  saveTimer = window.setTimeout(() => void api.setLayout(JSON.stringify(layoutDoc)).catch(() => {}), 300);
}

let saveTimer = 0;
/** A sheet's own layout: the first sheet's is the document's top level, which
 * also holds the other sheets', and they mustn't be saved inside a sheet. */
function ownLayout(layout: Partial<grid.Layout> & { sheets?: unknown }): Partial<grid.Layout> {
  const { sheets: _others, ...own } = layout;
  return own;
}

function layoutChanged(layout: grid.Layout) {
  if (board.sheet === null) layoutDoc = { ...ownLayout(layout), sheets: layoutDoc.sheets };
  else layoutDoc = { ...layoutDoc, sheets: { ...layoutDoc.sheets, [board.sheet]: ownLayout(layout) } };
  saveLayoutDoc();
  render(null);
}

// ---- sheets ----

/** Show a sheet in the grid, with its own layout. */
function showSheet(sheet: string | null) {
  if (editor.isEditing()) editor.cancel();
  if (terminalOpen()) void closeTerminal();
  // A filter belongs to the sheet it was set on.
  filter = "all";
  grid.setRowFilter(null);
  board.setSheet(sheet);
  grid.loadLayout(ownLayout(sheetLayout()));
  freezeDocumentGrid();
  inspector.renderInspector();
}

tabs.initTabs({
  show: showSheet,
  add: () => {
    const name = tabs.nextSheetName();
    void api
      .sheetEdit("add", name)
      .then(() => showSheet(name))
      .catch(oops);
  },
  rename: (from, to) => {
    void api
      .sheetEdit("rename", from, to)
      .then(() => {
        // Its layout goes with it; the first sheet's is the top level.
        const kept = layoutDoc.sheets?.[from];
        if (kept) {
          const sheets = { ...layoutDoc.sheets };
          delete sheets[from];
          layoutDoc = { ...layoutDoc, sheets: { ...sheets, [to]: kept } };
          saveLayoutDoc();
        }
        if (board.sheet === from) showSheet(to);
        else tabs.renderTabs(true);
      })
      .catch((e) => {
        oops(e);
        tabs.renderTabs(true);
      });
  },
  remove: (sheet) => {
    void api
      .sheetEdit("delete", sheet)
      .then(() => {
        if (board.sheet === sheet) showSheet(null);
      })
      .catch(oops);
  },
  move: (sheet, index) => void api.sheetEdit("move", sheet, undefined, index).catch(oops),
  duplicate: (sheet) => {
    void chrome.promptText(`Duplicate ${sheet}`, "the new sheet's name", `${sheet} copy`).then((to) => {
      to = to?.trim() ?? "";
      if (!to) return;
      void api
        .sheetEdit("duplicate", sheet, to)
        .then(() => {
          // The copy keeps the column widths and frozen panes it was laid out with.
          const kept = sheet === board.sheets.first ? null : layoutDoc.sheets?.[sheet];
          if (kept) {
            layoutDoc = { ...layoutDoc, sheets: { ...layoutDoc.sheets, [to]: kept } };
            saveLayoutDoc();
          }
          showSheet(to);
        })
        .catch(oops);
    });
  },
});

async function loadLayout() {
  layoutLoaded = false;
  const json = await api.getLayout().catch(() => "");
  let parsed: LayoutDoc = {};
  try {
    if (json) parsed = JSON.parse(json);
  } catch {
    parsed = {};
  }
  // Earlier versions could save the whole document inside a sheet's entry.
  const sheets = Object.fromEntries(Object.entries(parsed.sheets ?? {}).map(([name, l]) => [name, ownLayout(l)]));
  layoutDoc = { ...parsed, sheets };
  grid.loadLayout(ownLayout(sheetLayout()));
  layoutLoaded = true;
  freezeDocumentGrid();
}

// ---- find ----

let findHits: string[] = [];
let findAt = 0;

function findOpen(): boolean {
  return !$("findbar").hidden;
}

function openFind() {
  $("findbar").hidden = false;
  const input = $("find") as HTMLInputElement;
  input.focus();
  input.select();
  refreshFind(false);
}

function closeFind() {
  $("findbar").hidden = true;
  findHits = [];
  grid.setFindMatches(null, null);
  $("grid-wrap").focus({ preventScroll: true });
}

/** Recompute matches for the query; optionally jump to the first. */
function refreshFind(jump: boolean) {
  const q = ($("find") as HTMLInputElement).value.toLowerCase();
  findHits = q
    ? board
        .everywhere()
        .filter((c) => c.value.toLowerCase().includes(q) || c.name.toLowerCase().includes(q))
        .map((c) => c.name)
        .sort((a, b) => {
          const ca = a1Coord(a);
          const cb = a1Coord(b);
          if (ca && cb) return ca[1] - cb[1] || ca[0] - cb[0];
          return (ca ? 0 : 1) - (cb ? 0 : 1) || a.localeCompare(b);
        })
    : [];
  if (findAt >= findHits.length) findAt = 0;
  $("find-count").textContent = q ? (findHits.length ? `${findAt + 1} of ${findHits.length}` : "no matches") : "";
  grid.setFindMatches(findHits.length ? new Set(findHits) : null, findHits[findAt] ?? null);
  if (jump && findHits[findAt]) {
    board.select(findHits[findAt]);
    grid.scrollIntoView(findHits[findAt]);
  }
}

function findStep(delta: number) {
  if (!findHits.length) return;
  findAt = (findAt + delta + findHits.length) % findHits.length;
  refreshFind(true);
}

// ---- clipboard ----

function copySelection(e: ClipboardEvent) {
  const r = board.selectionRect();
  const names = board.selectedNames();
  if (names.length === 0 || !e.clipboardData) return;
  const lines: string[] = [];
  const block: Array<Array<string | null>> = [];
  const values: Array<Array<string | null>> = [];
  if (r) {
    for (let row = r[1]; row <= r[3]; row++) {
      const vals: string[] = [];
      const srcs: Array<string | null> = [];
      const whole: Array<string | null> = [];
      for (let col = r[0]; col <= r[2]; col++) {
        const c = board.get(a1Name(col, row));
        vals.push(c ? c.value.replace(/\t/g, " ").replace(/\n/g, " ") : "");
        srcs.push(c ? (c.source ?? c.value) : null);
        whole.push(c ? c.value : null);
      }
      lines.push(vals.join("\t"));
      block.push(srcs);
      values.push(whole);
    }
  } else {
    const c = board.get(names[0]);
    lines.push(c?.value ?? "");
    block.push([c ? (c.source ?? c.value) : null]);
    values.push([c ? c.value : null]);
  }
  const text = lines.join("\n");
  e.clipboardData.setData("text/plain", text);
  e.preventDefault();
  copied = { origin: r ? a1Name(r[0], r[1]) : names[0], text, block, values };
}

function pasteSelection(e: ClipboardEvent) {
  const r = board.selectionRect();
  const text = e.clipboardData?.getData("text/plain");
  if (!r || !text) return;
  e.preventDefault();
  const at = a1Name(r[0], r[1]);
  if (copied && copied.text === text) {
    void run(api.pasteCells(at, copied.origin, copied.block));
  } else {
    void run(api.pasteText(at, text));
  }
}

/** ⇧⌘V: paste copied cells as plain values, never the formula or agent behind them. */
function pasteValues() {
  const r = board.selectionRect();
  if (!r) return;
  const at = a1Name(r[0], r[1]);
  if (copied) {
    void run(api.pasteValues(at, copied.values));
    return;
  }
  void navigator.clipboard
    .readText()
    .then((text) => (text ? run(api.pasteText(at, text)) : undefined))
    .catch(oops);
}

/** The selection as a Markdown table: first row is the header. */
function copyMarkdown() {
  const r = board.selectionRect();
  if (!r) return;
  const cellText = (col: number, row: number) =>
    (board.get(a1Name(col, row))?.value ?? "").replace(/\|/g, "\\|").replace(/\n+/g, " ").trim();
  const rows: string[] = [];
  for (let row = r[1]; row <= r[3]; row++) {
    const cells: string[] = [];
    for (let col = r[0]; col <= r[2]; col++) cells.push(cellText(col, row));
    rows.push(`| ${cells.join(" | ")} |`);
    if (row === r[1]) rows.push(`|${" --- |".repeat(r[2] - r[0] + 1)}`);
  }
  void run(api.copyText(rows.join("\n"))).then(() => {
    flash = `copied ${r[3] - r[1] + 1} rows as a Markdown table`;
    inspector.renderInspector();
  });
}

// ---- keyboard (grid focus) ----

function onKey(e: KeyboardEvent) {
  if (editor.isEditing() || terminalOpen() || chrome.overlayOpen()) return;
  if (typingInField()) return;
  if (editor.composingKey(e)) return; // an input method's key, composing in the waiting input
  const target = navTarget(e);
  if (target) {
    e.preventDefault();
    moveSelection(target, e.shiftKey);
    return;
  }
  if (selectLine(e)) {
    e.preventDefault();
    return;
  }
  if (e.metaKey || e.ctrlKey) return; // shortcuts arrive as menu events
  if (reviewKey(e)) {
    e.preventDefault();
    return;
  }
  if (e.key === "Escape" && grid.cardPinned()) {
    e.preventDefault();
    grid.closeCard();
    return;
  }
  const { active, end } = board.selection;
  const co = active ? a1Coord(active) : null;
  if (co && e.key.startsWith("Arrow")) {
    e.preventDefault();
    const from = e.shiftKey && end ? a1Coord(end)! : co;
    const [c, r] = from;
    let next: [number, number] =
      e.key === "ArrowUp" ? [c, Math.max(1, r - 1)]
      : e.key === "ArrowDown" ? [c, r + 1]
      : e.key === "ArrowLeft" ? [Math.max(1, c - 1), r]
      : [c + 1, r];
    if (e.altKey) next = jump(from, e.key);
    moveSelection(next, e.shiftKey);
    return;
  }
  if (!active) return;
  const cell = board.active;
  if (cell?.decision && /^[1-9]$/.test(e.key) && !e.altKey) {
    // With a question on the selected cell, a digit picks that option.
    const i = Number(e.key) - 1;
    e.preventDefault();
    if (i < cell.decision.options.length) void run(api.resolveIndex(active, i));
    return;
  }
  if (e.key === "Enter" || e.key === "F2") {
    e.preventDefault();
    if (cell?.decision) {
      // Enter takes the recommended option only; with none, nothing is chosen.
      const rec = cell.decision.recommended ?? null;
      if (rec !== null) void run(api.resolveIndex(active, rec));
      else {
        flash = "No option is recommended; press its number or click it";
        inspector.renderInspector();
      }
    } else editor.beginEdit(active);
  } else if (e.key === "Tab") {
    e.preventDefault();
    if (co) select(a1Name(Math.max(1, co[0] + (e.shiftKey ? -1 : 1)), co[1]));
  } else if (e.key === "Escape") {
    if (end) board.select(active);
  } else if (e.key === "Delete" || e.key === "Backspace") {
    e.preventDefault();
    deleteSelected();
  } else if (e.key === " " && !e.shiftKey && !e.altKey && grid.expandCell(active)) {
    // Space previews a filled cell; Enter or typing edits.
    e.preventDefault();
  } else if (editor.isWaiting() && (e.key.length === 1 || e.key === "Dead")) {
    // The waiting input on the cell takes the key, so an input method or an accent
    // key composes there; its text starts the edit, replacing the cell's content.
  } else if (e.key.length === 1 && !e.altKey) {
    // Typing replaces the cell's content.
    e.preventDefault();
    editor.beginEdit(active, e.key);
  }
}

/** Move the selection to a cell, or with `extend` stretch it there from the active cell. */
function moveSelection(to: [number, number], extend: boolean) {
  const name = a1Name(to[0], to[1]);
  const active = board.selection.active;
  if (extend && active) {
    board.select(active, name);
    grid.scrollIntoView(name);
  } else {
    select(name);
  }
}

/** The keys Sheets and Excel move by, beyond the arrows: ⌘-arrow to the edge of the
 * data, Home and End along the row, ⌘Home and ⌘End to the corners, Page Up and
 * Page Down by a screen. Null for any other key. */
function navTarget(e: KeyboardEvent): [number, number] | null {
  const { active, end } = board.selection;
  const co = active ? a1Coord(active) : null;
  if (!co || e.altKey) return null;
  const from = e.shiftKey && end ? (a1Coord(end) ?? co) : co;
  const mod = e.metaKey || e.ctrlKey;
  const [maxC, maxR] = board.extent();
  if (mod && e.key.startsWith("Arrow")) return jump(from, e.key);
  if (e.key === "Home") return mod ? [1, 1] : [1, from[1]];
  if (e.key === "End") return mod ? [Math.max(1, maxC), Math.max(1, maxR)] : [Math.max(1, maxC), from[1]];
  if (mod) return null;
  if (e.key === "PageDown") return [from[0], from[1] + grid.pageRows()];
  if (e.key === "PageUp") return [from[0], Math.max(1, from[1] - grid.pageRows())];
  return null;
}

/** ⇧Space selects the active cell's row, ⌃Space its column, across the filled block. */
function selectLine(e: KeyboardEvent): boolean {
  if (e.key !== " " || e.altKey || e.metaKey || e.shiftKey === e.ctrlKey) return false;
  const co = board.selection.active ? a1Coord(board.selection.active) : null;
  if (!co) return false;
  const [maxC, maxR] = board.extent();
  if (e.shiftKey) select(a1Name(1, co[1]), a1Name(Math.max(1, maxC), co[1]));
  else select(a1Name(co[0], 1), a1Name(co[0], Math.max(1, maxR)));
  return true;
}

/** ⌥-arrow: to the edge of the filled block, or the next filled cell. */
function jump(from: [number, number], key: string): [number, number] {
  const [dc, dr] = key === "ArrowUp" ? [0, -1] : key === "ArrowDown" ? [0, 1] : key === "ArrowLeft" ? [-1, 0] : [1, 0];
  const filled = (c: number, r: number) => c >= 1 && r >= 1 && board.cells.has(a1Name(c, r));
  let [c, r] = from;
  const [maxC, maxR] = board.extent();
  if (filled(c, r) && filled(c + dc, r + dr)) {
    while (filled(c + dc, r + dr)) [c, r] = [c + dc, r + dr];
    return [c, r];
  }
  do [c, r] = [c + dc, r + dr];
  while (c >= 1 && r >= 1 && c <= maxC + 1 && r <= maxR + 1 && !filled(c, r));
  if (c < 1 || r < 1) return from;
  if (filled(c, r)) return [c, r];
  return [dc > 0 ? Math.max(1, maxC) : dc < 0 ? 1 : c, dr > 0 ? Math.max(1, maxR) : dr < 0 ? 1 : r];
}

function typingInField(): boolean {
  if (editor.isWaiting()) return false;
  const tag = document.activeElement?.tagName;
  return tag === "INPUT" || tag === "TEXTAREA";
}

// ---- context menus ----

function entry(id: string): chrome.MenuEntry | null {
  const act = actions.actionById(id);
  return act
    ? { label: act.label, shortcut: act.shortcut, disabled: act.enabled ? !act.enabled() : false, run: act.run }
    : null;
}

function contextMenu(x: number, y: number) {
  const cell = board.active;
  const multi = board.selectedNames().length > 1;
  const busy = cell && (stateKind(cell.state) === "computing" || stateKind(cell.state) === "queued");
  chrome.showContextMenu(x, y, [
    busy ? entry("cell.stop") : multi ? entry("cell.run_selection") : entry("cell.run"),
    ...(board.selectedNames().some((n) => board.get(n)?.kind === "agent_call") ? [entry("cell.driver_answer")] : []),
    cell?.kind === "agent_call" ? entry("agent.session") : entry("cell.terminal"),
    null,
    { label: "Copy", shortcut: "⌘C", run: () => document.execCommand("copy") },
    { label: "Paste", shortcut: "⌘V", run: () => document.execCommand("paste") },
    entry("edit.copy_markdown"),
    entry("edit.delete"),
    null,
    entry("cell.verify"),
    entry("cell.flag"),
    cell?.cite ? { label: "Show Source", shortcut: "⌥⌘S", run: () => side.showSource(cell) } : entry("cell.sparkline"),
    null,
    entry("format.more"),
    entry("format.clear"),
    null,
    entry("edit.fill_down"),
    entry("edit.fill_right"),
    null,
    entry("view.filter_value"),
    entry("view.freeze"),
  ]);
}

function headerMenu(x: number, y: number, axis: "rows" | "cols", index: number) {
  // Right-clicking a header outside the selection selects that row/column.
  const r = board.selectionRect();
  const within = r && (axis === "rows" ? index >= r[1] && index <= r[3] : index >= r[0] && index <= r[2]);
  if (!within) {
    const [maxC, maxR] = board.extent();
    if (axis === "rows") select(a1Name(1, index), a1Name(Math.max(1, maxC), index));
    else select(a1Name(index, 1), a1Name(index, Math.max(1, maxR)));
  }
  chrome.showContextMenu(
    x,
    y,
    axis === "rows"
      ? [entry("edit.insert_row"), entry("edit.insert_row_below"), entry("edit.delete_rows"), null, entry("format.header"), entry("format.more"), entry("cell.run_selection"), entry("view.freeze")]
      : [
          entry("cell.run_column"),
          entry("cell.add_column"),
          null,
          entry("edit.sort_asc"),
          entry("edit.sort_desc"),
          null,
          entry("edit.insert_col"),
          entry("edit.insert_col_right"),
          entry("edit.delete_cols"),
          null,
          entry("format.number"),
          entry("format.more"),
          entry("format.clear"),
          null,
          entry("cell.run_selection"),
          entry("view.filter_column"),
          entry("view.freeze"),
        ],
  );
}

// ---- boot ----

grid.initGrid({
  select,
  fillTo: (to) => {
    // Dragged past the selection: fill from its top-left to the pointer, then select
    // the filled block.
    const r = board.selectionRect();
    if (!r) return;
    const from = a1Name(r[0], r[1]);
    const toCo = a1Coord(to)!;
    const dir = toCo[1] > r[3] ? "down" : "right";
    void run(api.fill(from, to, dir)).then(() => board.select(from, to));
  },
  edit: (name) => {
    select(name);
    editor.beginEdit(name);
  },
  pickingRefs: editor.pickingRefs,
  commitOpenEdit: editor.commitOpen,
  pickRef: (anchor, current, done) => {
    const a = a1Coord(anchor)!;
    const b = a1Coord(current)!;
    const text =
      anchor === current
        ? `{${anchor}}`
        : `{${a1Name(Math.min(a[0], b[0]), Math.min(a[1], b[1]))}:${a1Name(Math.max(a[0], b[0]), Math.max(a[1], b[1]))}}`;
    editor.insertRef(text, done);
  },
  contextMenu: (x, y) => contextMenu(x, y),
  headerMenu,
  layoutChanged,
});

editor.initEditor({
  commit: commitEdit,
  cancelled: () => {
    $("grid-wrap").focus({ preventScroll: true });
    inspector.renderInspector();
  },
});

inspector.initInspector({
  runColumn,
  flash: () => (flash ? { text: flash, error: flashIsError } : null),
  review: reviewStatus,
  endReview: () => endReview(),
  runWithConfirm,
  runSelection,
  setMark: (kind) => setMark(kind),
  flagWithNote: flagSelected,
  deleteSelected,
  select,
  run: (p) => void run(p),
  formatSelection: () => format("more"),
  showSource: (cell) => side.showSource(cell),
  addQuestionColumn: () => void addQuestionColumn(),
  answerTypeSelection: () => void answerTypeSelection(),
});

side.setFormatView(() => renderFormatView(true));
side.initSide({
  newCell,
  select,
  // A double click on a named row edits it there, as a grid cell is edited in place.
  edit: (name) => {
    if (editor.isEditing()) return;
    select(name);
    editor.beginEdit(name);
  },
  oops,
  review: startReview,
  showSession: (name) => {
    select(name);
    const cell = board.get(name);
    if (cell?.kind === "agent_call") void inspector.openAgentSession(cell);
  },
});
// Elapsed timers in the needs-you panel and on working cells tick on their own.
window.setInterval(() => {
  side.tickTray();
  grid.tickElapsed();
}, 1000);

actions.initActions({
  editSelected,
  newCell,
  runSelected,
  runSelection,
  driverAnswer,
  driverRunning: side.driverRunning,
  boardScripts,
  stopSelected,
  stopAll,
  setBudget,
  copyMarkdown,
  pasteValues,
  find: openFind,
  toggleWrap,
  toggleNotify,
  freeze,
  unfreeze,
  setFilter,
  filterToValue,
  filterColumn: () => void filterColumn(),
  restructure,
  sort: sortByActiveColumn,
  format,
  answerTypeSelection: () => void answerTypeSelection(),
  openNew: () => openWelcome("New board"),
  startNew: (fill) => void startNew(fill),
  addQuestionColumn: () => void addQuestionColumn(),
  runColumn,
  toggleSource,
  openTerminalSelected: () => {
    const cell = board.active;
    // An agent cell's terminal is its session; a plain PTY on the same id would kill it.
    if (cell?.kind === "agent_call") void inspector.openAgentSession(cell);
    else if (cell && isRunnable(cell.kind)) void inspector.openCellTerminal(cell);
  },
  openSessionSelected: () => {
    const cell = board.active;
    if (cell?.kind === "agent_call") void inspector.openAgentSession(cell);
  },
  endSessionSelected: () => {
    const cell = board.active;
    if (cell?.session) void run(api.sessionEnd(cell.name));
  },
  handoffSelected: () => {
    const cell = board.active;
    if (cell?.kind === "agent_call") void run(api.sessionHandoff(cell.name, true));
  },
  showDriver: () => side.showTray("driver"),
  toggleWideDriver: () => side.toggleWideDriver(),
  notifySelected: () => {
    const cell = board.active;
    if (cell?.stale_inputs?.length) void run(api.sessionNotify(cell.name));
  },
  toggleVerified,
  flagSelected,
  nextUnverified,
  verifyAndNext,
  startReview,
  toggleSpark: () => {
    const cell = board.active;
    if (cell) void run(api.setSeries(cell.name, !cell.series));
  },
  deleteSelected,
  fillSelection,
  undo: () => {
    if (typingInField()) document.execCommand("undo");
    else void api.undo().then((n) => n && select(board.local(n))).catch(oops);
  },
  redo: () => {
    if (typingInField()) document.execCommand("redo");
    else void api.redo().then((n) => n && select(board.local(n))).catch(oops);
  },
  selectAll: () => {
    if (typingInField() || terminalOpen() || chrome.overlayOpen()) document.execCommand("selectAll");
    else {
      // The filled block from A1, as in Sheets; the view stays where it is.
      const [maxC, maxR] = board.extent();
      board.select("A1", a1Name(Math.max(1, maxC), Math.max(1, maxR)));
    }
  },
  showDecisions: () => {
    side.showTray("decisions");
    // Select the first question so number keys answer it.
    const first = board.everywhere().find((c) => c.decision);
    if (first) select(first.name);
  },
  oops,
});
chrome.initToolbar(actions.actionById);
chrome.setErrorReporter(oops);

// Find bar.
$("find").addEventListener("input", () => {
  findAt = 0;
  refreshFind(true);
});
$("find").addEventListener("keydown", (e) => {
  if (e.key === "Enter") {
    e.preventDefault();
    findStep(e.shiftKey ? -1 : 1);
  } else if (e.key === "Escape") {
    e.preventDefault();
    closeFind();
  }
});
$("find-next").onclick = () => findStep(1);
$("find-prev").onclick = () => findStep(-1);
$("find-close").onclick = closeFind;

document.addEventListener("keydown", onKey);
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") {
    if (chrome.overlayOpen()) chrome.closeOverlay();
    chrome.closeContextMenu();
  }
});
document.addEventListener("mousedown", (e) => {
  if (!(e.target as HTMLElement).closest("#ctx")) chrome.closeContextMenu();
});
document.addEventListener("copy", (e) => {
  if (!typingInField() && !terminalOpen()) copySelection(e);
});
document.addEventListener("cut", (e) => {
  if (!typingInField() && !terminalOpen()) {
    copySelection(e);
    deleteSelected();
  }
});
document.addEventListener("paste", (e) => {
  if (!typingInField() && !terminalOpen()) pasteSelection(e);
});
$("overlay").addEventListener("click", (e) => {
  if (e.target === $("overlay")) chrome.closeOverlay();
});

// Splitters set the CSS vars that size the sidebar and inspector; sizes persist,
// and a focused splitter takes arrow keys.
function setupSplitters() {
  const root = document.documentElement.style;
  const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));
  const KEY = "recalc.panes";
  try {
    const saved = JSON.parse(localStorage.getItem(KEY) ?? "{}") as Record<string, string>;
    for (const v of ["--side-w", "--inspector-h"]) if (/^\d+px$/.test(saved[v] ?? "")) root.setProperty(v, saved[v]);
  } catch {
    // No storage (private window, preview): keep the defaults.
  }
  const remember = () => {
    try {
      localStorage.setItem(KEY, JSON.stringify({ "--side-w": root.getPropertyValue("--side-w"), "--inspector-h": root.getPropertyValue("--inspector-h") }));
    } catch {
      // Size just isn't persisted.
    }
  };
  const sideW = (px: number) => root.setProperty("--side-w", `${clamp(px, 220, 680)}px`);
  const inspH = (px: number) => root.setProperty("--inspector-h", `${clamp(px, 90, window.innerHeight - 220)}px`);
  const size = (v: string, fallback: number) => parseInt(root.getPropertyValue(v)) || fallback;
  const keys = (el: HTMLElement, label: string, step: (dir: number) => void, back: string, fwd: string) => {
    el.tabIndex = 0;
    el.setAttribute("role", "separator");
    el.setAttribute("aria-label", label);
    el.addEventListener("keydown", (e) => {
      const dir = e.key === back ? -1 : e.key === fwd ? 1 : 0;
      if (!dir) return;
      e.preventDefault();
      step(dir * (e.shiftKey ? 64 : 16));
      remember();
      render(null);
    });
  };
  keys($("v-split"), "Resize the side panel", (d) => sideW(size("--side-w", 300) - d), "ArrowLeft", "ArrowRight");
  keys($("h-split"), "Resize the inspector", (d) => inspH(size("--inspector-h", 240) - d), "ArrowUp", "ArrowDown");
  const drag = (el: HTMLElement, onMove: (e: MouseEvent) => void) => {
    el.addEventListener("mousedown", (down) => {
      down.preventDefault();
      document.body.style.cursor = getComputedStyle(el).cursor;
      const move = (e: MouseEvent) => onMove(e);
      const up = () => {
        document.removeEventListener("mousemove", move);
        document.removeEventListener("mouseup", up);
        document.body.style.cursor = "";
        remember();
        render(null);
      };
      document.addEventListener("mousemove", move);
      document.addEventListener("mouseup", up);
    });
  };
  drag($("v-split"), (e) => sideW(window.innerWidth - e.clientX));
  drag($("h-split"), (e) => inspH(window.innerHeight - e.clientY));
  // Double-click the inspector's edge to toggle between compact and half-window.
  $("h-split").addEventListener("dblclick", () => {
    const current = parseInt(root.getPropertyValue("--inspector-h")) || 0;
    const half = Math.round(window.innerHeight * 0.5);
    root.setProperty("--inspector-h", `${current >= half - 10 ? 240 : half}px`);
    remember();
    render(null);
  });
}
setupSplitters();
// The overlay title bar leaves room for the traffic lights only on macOS.
if (/Mac/i.test(navigator.platform)) document.body.classList.add("mac");

board.onChange(render);
void api.listen<Snapshot>("board", (e) => {
  if (e.payload.full && e.payload.dir !== board.dir) {
    actions.resetMenuSync();
    filter = "all";
    grid.setRowFilter(null);
    side.driverBoardChanged();
    board.apply(e.payload);
    void loadLayout();
    return;
  }
  const before = new Map(e.payload.cells.map((c) => [c.name, board.all.get(c.name)?.state]));
  board.apply(e.payload);
  for (const c of e.payload.cells) {
    const was = before.get(c.name);
    if (was !== "computing" && was !== "queued") continue;
    const now = stateKind(c.state);
    if (now === "ok") announced.push(`${c.name} answered`);
    else if (now === "error") announced.push(`${c.name} failed`);
  }
  if (announced.length && !announceTimer) announceTimer = window.setTimeout(announce, 3000);
});

// Announce completions through a polite live region, throttled so a table landing
// isn't one announcement per cell.
const announced: string[] = [];
let announceTimer = 0;
function announce() {
  announceTimer = 0;
  const said = announced.length <= 3 ? announced.join(", ") : `${announced.length} cells finished, ${announced.filter((s) => s.endsWith("failed")).length} failed`;
  announced.length = 0;
  const el = document.getElementById("announce");
  if (el) el.textContent = said;
}
// An agent asked to show something (open_session / show_cell); the backend already
// started the session, so bring it forward.
void api.listen<string>("show-session", (e) => {
  const cell = board.get(board.local(e.payload));
  if (cell?.kind !== "agent_call") return;
  select(cell.name);
  void inspector.openAgentSession(cell);
});
void api.listen<string>("show-cell", (e) => {
  const name = board.local(e.payload);
  if (board.get(name)) select(name);
});
void api.listen<string>("menu", (e) => actions.runAction(e.payload));
// A file Finder handed the app that didn't open.
void api.listen<string>("notice", (e) => chrome.notice("Couldn't open the board", e.payload));
void api.getBoard().then((snap) => {
  board.apply(snap);
  void loadLayout();
});
onboard.initUpdates();
void api.launchNotices().then((notes) => {
  if (notes.length) chrome.notice("Move recalc to Applications", notes.join("\n\n"));
}).catch(() => {});
