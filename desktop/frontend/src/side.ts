// Side panels: named cells (off the A1 grid) and the tray (driver, needs-you
// queue, activity log, source pane). Snoozed needs return on a timer or when
// their cell changes.

import { a1Coord, a1Name } from "./a1.js";
import { feed, feedRow, tickFeed } from "./activity.js";
import { cellFace, getLayout, setLayout } from "./grid.js";
import type { Snooze } from "./grid.js";
import { board } from "./state.js";
import * as api from "./tauri.js";
import { DRIVER_ID, attachDriver, closeTerminal, driverViewOpen, showTerminal } from "./terminal.js";
import type { Cell } from "./types.js";
import { actorShort, agentChoice, agentProduct, API_PRICES, costLabel, elapsedLabel, errorMsg, firstLineOf, isRunnable, permissionAsk, permissionRisks, sessionWord, stateKind } from "./types.js";

const $ = (id: string) => document.getElementById(id)!;

export type TrayTab = "activity" | "decisions" | "source" | "driver";
let trayTab: TrayTab = "driver";
/** What the side panel shows: Agents (the tray and named cells), Format, or
 * nothing. The switch at the toolbar's right end picks; clicking the showing
 * one hides the panel. Agents first, as the board opens. */
export type SideView = "agents" | "format" | "hidden";
let sideView: SideView = "agents";
let onFormatView: () => void = () => {};
let namedOpen = true;
/** Widen the panel while the Driver tab shows; its terminal needs ~80 columns. */
const WIDE_KEY = "recalc.wideDriver";
let wideDriver = (() => {
  try {
    return localStorage.getItem(WIDE_KEY) === "1";
  } catch {
    return false;
  }
})();

export function toggleWideDriver() {
  wideDriver = !wideDriver;
  try {
    localStorage.setItem(WIDE_KEY, wideDriver ? "1" : "0");
  } catch {
    // Storage unavailable: the choice lasts for this window only.
  }
  if (trayTab !== "driver") showTray("driver");
  else renderTray();
}

export function isWideDriver(): boolean {
  return wideDriver;
}
let onNewCell: () => void = () => {};
let onReview: () => void = () => {};
let onSelect: (name: string) => void = () => {};
let onEdit: (name: string) => void = () => {};
/** The last press on a named row, to tell a double click across a redraw. */
let lastPress: { name: string; at: number } | null = null;
let onShowSession: (name: string) => void = () => {};
let onError: (e: unknown) => void = () => {};
/** Key of the last tray render; unchanged keys skip the rebuild to keep scroll and hover. */
let trayKey = "";

/** Source pane contents, and one cached fetch per file. */
let shown: { file: string; quote: string | null } | null = null;
const sources = new Map<string, Promise<api.Source>>();

export function initSide(h: { newCell(): void; select(name: string): void; edit(name: string): void; oops(e: unknown): void; review(): void; showSession(name: string): void }) {
  onReview = h.review;
  onShowSession = h.showSession;
  onNewCell = h.newCell;
  onSelect = h.select;
  onEdit = h.edit;
  onError = h.oops;
  $("tab-activity").onclick = () => showTray("activity");
  $("tab-decisions").onclick = () => showTray("decisions");
  $("tab-source").onclick = () => showTray("source");
  $("tab-driver").onclick = () => showTray("driver");
  $("switch-agents").onclick = () => toggleSide("agents");
  $("switch-format").onclick = () => toggleSide("format");
  void refreshDriver();
  $("named-head").onclick = () => {
    namedOpen = !namedOpen;
    $("side").classList.toggle("named-closed", !namedOpen);
    renderNamed();
  };
  void api.listen<{ id: string }>("pty-exit", (e) => {
    if (e.payload.id === DRIVER_ID) void refreshDriver();
  });
  // A press anywhere else in between means it wasn't a double click.
  document.addEventListener("mousedown", (e) => {
    if (!(e.target as HTMLElement).closest("#named")) lastPress = null;
  }, true);
  $("named").addEventListener("mousedown", (e) => {
    // A click in the row's own editor places the caret.
    if ((e.target as HTMLElement).closest("#cell-editor")) return;
    const name = (e.target as HTMLElement).closest<HTMLElement>(".named-row")?.dataset.name;
    if (!name) return;
    // Selecting redraws the row, so the browser's dblclick never comes: a
    // second press on the same name soon after is the double click.
    const now = performance.now();
    const double = lastPress?.name === name && now - lastPress.at < 500;
    lastPress = double ? null : { name, at: now };
    if (double) {
      e.preventDefault();
      onEdit(name);
    } else onSelect(name);
  });
  $("tray").addEventListener("click", (e) => {
    const name = (e.target as HTMLElement).closest<HTMLElement>(".tray-row")?.dataset.name;
    if (name) onSelect(name);
  });
}

export function showTray(tab: TrayTab) {
  trayTab = tab;
  if (tab === "source") followSelection();
  if (tab === "driver") void refreshDriver();
  if (sideView !== "agents") showSide("agents");
  else renderTray();
}

/** Show a side panel view, or hide the panel. */
export function showSide(view: SideView) {
  sideView = view;
  document.querySelector("main")?.classList.toggle("side-hidden", view === "hidden");
  $("side").classList.toggle("show-format", view === "format");
  for (const v of ["agents", "format"] as const) {
    const b = $(`switch-${v}`);
    b.classList.toggle("on", view === v);
    b.setAttribute("aria-pressed", String(view === v));
  }
  renderTray();
  if (view === "format") onFormatView();
}

/** The switch: show `view`, or hide the panel if it's already showing. */
export function toggleSide(view: "agents" | "format") {
  showSide(sideView === view ? "hidden" : view);
}

export function sideShowing(): SideView {
  return sideView;
}

/** What to draw when the Format view opens (it lives in app.ts, beside format()). */
export function setFormatView(draw: () => void) {
  onFormatView = draw;
}

export function trayShowing(): TrayTab {
  return trayTab;
}

/** Open the source pane on a cell's citation, or on its row's document. */
export function showSource(cell: Cell | null) {
  trayTab = "source";
  shown = sourceFor(cell);
  if (sideView !== "agents") showSide("agents");
  else renderTray();
}

/** While open, the pane shows the selection's cited passage, or its row's document. */
export function followSelection() {
  if (trayTab !== "source") return;
  const next = sourceFor(board.active);
  if (next?.file === shown?.file && next?.quote === shown?.quote) return;
  shown = next;
  renderTray();
}

function sourceFor(cell: Cell | null): { file: string; quote: string | null } | null {
  if (!cell) return null;
  if (cell.cite) return { file: cell.cite.file, quote: cell.cite.quote };
  const co = a1Coord(cell.name);
  const doc = co ? board.get(a1Name(1, co[1])) : undefined;
  const first = doc?.value.split(/[,\n]/)[0].trim();
  if (first && /\.[A-Za-z0-9]+$/.test(first) && !first.includes(" ")) return { file: first, quote: null };
  return null;
}

export function renderNamed() {
  const box = $("named");
  // The row being edited stays put: taking the input out of the page would end the edit.
  const keep = box.querySelector<HTMLElement>(".named-row.editing");
  for (const el of [...box.children]) if (el !== keep) el.remove();
  const named = [...board.cells.values()]
    .filter((c) => !/^[A-Z]+[0-9]+$/.test(c.name))
    .sort((a, b) => a.name.localeCompare(b.name));
  $("named-count").textContent = named.length ? String(named.length) : "";
  $("named-head").classList.toggle("closed", !namedOpen);
  if (!namedOpen) return;
  const active = board.active;
  let pastKeep = false;
  const put = (el: HTMLElement) => {
    if (keep && !pastKeep) box.insertBefore(el, keep);
    else box.appendChild(el);
  };
  if (!named.length) {
    const help = document.createElement("div");
    help.className = "named-help";
    help.textContent = "Cells with a name instead of a place, like budget or status. Other cells use them as {budget}; agents read and write them by name.";
    put(help);
  }
  for (const cell of named) {
    if (keep && cell.name === keep.dataset.name) {
      pastKeep = true;
      continue;
    }
    const row = document.createElement("div");
    row.className = "named-row";
    if (cell.name === board.selection.active) row.classList.add("selected");
    if (active && active.deps.includes(cell.name)) row.classList.add("dep");
    row.dataset.name = cell.name;
    const nm = document.createElement("span");
    nm.className = "nm";
    nm.textContent = cell.name;
    const face = document.createElement("span");
    face.className = "face";
    cellFace(face, cell);
    row.append(nm, face);
    put(row);
  }
  // Its cell is gone (deleted elsewhere): so is the row.
  if (keep && !named.some((c) => c.name === keep.dataset.name)) keep.remove();
  const add = document.createElement("div");
  add.className = "named-row add";
  add.textContent = "+ New cell";
  add.title = "Make a cell with a name, such as budget or status, that other cells and agents refer to by name";
  add.onclick = onNewCell;
  box.appendChild(add);
}

const TABS: TrayTab[] = ["driver", "decisions", "activity", "source"];

// ---- notifications ----
//
// New needs while the window is unfocused raise a desktop notification,
// batched to one per NOTIFY_GAP_MS. Needs present at startup aren't announced.

const announced = new Set<string>();
let primed = false;
let waiting: Need[] = [];
let flushAt: number | null = null;
let lastShown = 0;
let named: string | null = null;
const NOTIFY_GAP_MS = 10_000;
const KIND_WORD: Record<NeedKind, string> = {
  ask: "question",
  suggest: "suggestion",
  stale: "inputs changed",
  error: "error",
  flag: "flagged",
  done: "finished",
  cite: "citation not found",
  waiting: "session waiting",
};

function noteNews() {
  const now = needs();
  const key = (n: Need) => `${n.kind}:${n.cell.name}:${n.since}`;
  const fresh = now.filter((n) => !announced.has(key(n)));
  for (const n of fresh) announced.add(key(n));
  if (!primed) {
    primed = true;
    return;
  }
  if (fresh.length === 0 || getLayout().notify === false || document.hasFocus()) return;
  waiting.push(...fresh);
  if (flushAt === null) flushAt = window.setTimeout(flushNews, Math.max(0, lastShown + NOTIFY_GAP_MS - Date.now()));
}

function flushNews() {
  flushAt = null;
  const news = waiting;
  waiting = [];
  if (news.length === 0 || document.hasFocus()) return;
  const first = news[0];
  const finished = news.every((n) => n.kind === "done");
  const title = finished
    ? news.length === 1
      ? `${first.cell.name} finished`
      : `${news.length} answers finished`
    : news.length === 1
      ? `${first.cell.name} needs you`
      : `${news.length} things need you`;
  const body =
    news.length === 1
      ? `${KIND_WORD[first.kind]}: ${firstLineOf(first.text)}`
      : news
          .slice(0, 3)
          .map((n) => `${n.cell.name} ${KIND_WORD[n.kind]}`)
          .join(" · ") + (news.length > 3 ? " …" : "");
  named = first.cell.name;
  lastShown = Date.now();
  void api.notify(title, body).catch(() => {});
}

window.addEventListener("focus", () => {
  if (!named) return;
  const name = named;
  named = null;
  onSelect(name);
  showTray("decisions");
});

export function renderTray() {
  noteNews();
  const waiting = needsAction().length;
  $("needs-count").textContent = waiting ? String(waiting) : "";
  // The switch carries the count too, so it shows while Format is open.
  $("agents-count").textContent = waiting ? String(waiting) : "";
  $("switch-agents").setAttribute("aria-label", waiting ? `Agents, ${waiting} need you` : "Agents");
  $("tab-decisions").setAttribute("aria-label", waiting ? `Needs you, ${waiting} waiting` : "Needs you");
  for (const t of TABS) {
    const tab = $(`tab-${t}`);
    tab.classList.toggle("active", trayTab === t);
    tab.setAttribute("aria-selected", String(trayTab === t));
  }
  $("tab-driver").classList.toggle("live", driverViewOpen() || (driverState?.status ?? "none") !== "none");
  $("side").classList.toggle("show-source", trayTab === "source");
  $("side").classList.toggle("show-driver", trayTab === "driver");
  document.querySelector("main")?.classList.toggle("wide-driver", wideDriver && trayTab === "driver" && sideView === "agents");
  const box = $("tray");
  box.setAttribute("aria-labelledby", `tab-${trayTab}`);
  if (trayTab === "source") {
    trayKey = "";
    renderSource(box);
    return;
  }
  if (trayTab === "driver") {
    trayKey = "";
    renderDriver(box);
    return;
  }
  const a = board.activity;
  const key = trayTab === "decisions" ? needsKey() : `activity|${a.length}|${a[0]?.at ?? ""}|${a[0]?.text ?? ""}|${a[a.length - 1]?.at ?? ""}|${board.cells.size}`;
  if (key === trayKey && box.firstChild) return;
  trayKey = key;
  const scroll = box.scrollTop;
  box.replaceChildren();
  if (trayTab === "decisions") {
    renderNeedsYou(box);
  } else {
    if (board.activity.length === 0) {
      const empty = document.createElement("div");
      empty.className = "tray-empty";
      empty.textContent = "What people and agents change on this board shows here.";
      box.appendChild(empty);
    }
    for (const r of feed(board.activity)) box.appendChild(feedRow(r, onSelect));
  }
  box.scrollTop = scroll;
}

/** Render key for the needs-you rows. Excludes wait times; tickTray updates those in place. */
function needsKey(): string {
  const parts: string[] = [`decisions|${shelfOpen}`];
  for (const n of needs()) {
    const d = n.kind === "ask" ? n.cell.decision : null;
    const opts = d ? `${d.options.map((o) => `${o.label}\u0001${o.preview ?? ""}`).join("\u0002")}|${d.recommended ?? ""}|${d.asker}` : "";
    const from = n.kind === "suggest" ? n.cell.pending_suggestions[n.cell.pending_suggestions.length - 1].from : "";
    parts.push(`n|${n.kind}|${n.cell.name}|${n.text}|${n.since}|${opts}|${from}|${n.cell.kind}`);
  }
  for (const c of working()) parts.push(`w|${c.name}|${stateKind(c.state)}|${c.kind}|${c.session ?? ""}|${c.activity ?? ""}`);
  for (const { need, until } of snoozed()) parts.push(`z|${need.kind}|${need.cell.name}|${need.text}|${until}`);
  return parts.join("\u0000");
}

// ---- the driver ----
//
// A board-level agent in the tray's terminal. Under tmux it outlives the
// window; without tmux it is a plain PTY that ends with the app.

let driverState: api.DriverStatus | null = null;
/** Reused across renders so the terminal element isn't rebuilt. */
const driverHead = document.createElement("div");
driverHead.className = "driver-head";
const driverHint = document.createElement("div");
driverHint.className = "driver-hint";
const driverHolder = document.createElement("div");
driverHolder.className = "driver-holder";

async function refreshDriver() {
  driverState = await api.driverStatus().catch(() => null);
  if (trayTab === "driver") renderDriver($("tray"));
}

function renderDriver(box: HTMLElement) {
  if (box.firstElementChild !== driverHead) box.replaceChildren(driverHead, driverHint, driverHolder);
  driverHead.replaceChildren();
  const who = document.createElement("span");
  who.className = "who";
  const status = driverState?.status ?? "none";
  const shown = driverViewOpen();
  const agentName = agentProduct;
  const current = driverState?.agent ?? "";
  if (status === "none" && !shown && (driverState?.choices.length ?? 0) > 1) {
    // The agent can only be changed while the driver isn't running.
    who.textContent = "Driver · ";
    const pick = document.createElement("select");
    pick.className = "driver-pick";
    pick.setAttribute("aria-label", "Agent that drives this board");
    pick.title = "Choose the agent that drives this board; it's saved in .recalc/config.toml";
    for (const c of driverState!.choices) {
      const o = document.createElement("option");
      o.value = c;
      o.textContent = agentChoice(c);
      o.selected = c === current;
      pick.appendChild(o);
    }
    pick.onchange = () =>
      void api
        .setDriver(pick.value)
        .then(refreshDriver)
        .catch((e) => {
          flashDriver(e);
          void refreshDriver();
        });
    who.appendChild(pick);
  } else {
    who.textContent = `Driver · ${current ? agentName(current) : "…"}`;
  }
  driverHead.appendChild(who);
  const dot = document.createElement("span");
  dot.className = "dot" + (status === "detached" && !shown ? " detached" : "");
  // The view can outlive its process; keep the terminal so its exit output stays readable.
  const ended = shown && status === "none";
  dot.textContent = ended ? "○ ended" : shown ? "● live" : status === "detached" ? "● running in the background" : status === "running" ? "● running" : "○ not started";
  if (ended) dot.title = "The driver's process ended; its terminal says why. Start it again when you're ready.";
  driverHead.appendChild(dot);
  const used = board.agents.driver_cost ?? 0;
  if (used > 0) {
    const u = document.createElement("span");
    u.className = "driver-used";
    u.textContent = `${costLabel(used)} used`;
    u.title = `What the Driver's ended sessions on this board used, as Claude Code recorded them. ${API_PRICES}`;
    driverHead.appendChild(u);
  }
  const act = (label: string, title: string, on: () => unknown) => {
    const b = document.createElement("button");
    b.className = "act small";
    b.textContent = label;
    b.title = title;
    b.onclick = () => void on();
    driverHead.appendChild(b);
  };
  if (!shown || ended) {
    if (status === "none" && driverState?.resumable) {
      act("Resume", "Continue the driver's last conversation", () => startDriver());
      act("Start new", "Start the driver in a new conversation; it reads the board, not the old conversation", () => startDriver(true));
    } else {
      act(status === "none" ? "Start" : "Show", status === "none" ? "Start the driver: an agent connected to this board that works it for you" : "Show the running driver here", () => startDriver());
    }
  }
  act(wideDriver ? "Narrow" : "Widen", wideDriver ? "Give the grid its room back (⌥⌘D)" : "Widen the panel so the driver's terminal has room (⌥⌘D)", toggleWideDriver);
  if (status !== "none" || (shown && !ended)) {
    act("Open in Terminal", "Join the driver in a terminal window; both show the same agent", () => api.driverHandoff(true).catch(flashDriver));
    act("End", "Stop the driver", async () => {
      await api.driverEnd().catch(flashDriver);
      await closeTerminal(DRIVER_ID);
      await refreshDriver();
    });
  }
  driverHint.hidden = shown && !driverError;
  if (driverError) {
    driverHint.textContent = driverError;
  } else if (!shown) {
    driverHint.textContent =
      status === "none"
        ? "An assistant that sets up and runs this board for you. Press Start, then say what you need in plain words (\u201cabstract the leases in documents/\u201d). It fills the cells and tells you what to check. Only you can verify answers."
        : "The driver is running in the background. Show it here, or join it from a terminal.";
    if (driverState && !driverState.tmux) {
      driverHint.textContent += " It stops when you close this window. To keep it running, install tmux (brew install tmux).";
    }
  }
}

/** Whether a Driver is running, here or in the background. */
export function driverRunning(): boolean {
  return driverViewOpen() || (driverState?.status ?? "none") !== "none";
}

/** Show the Driver tab with its terminal attached here (pressing Show when it
 * runs in the background). False when no Driver is running. */
export async function ensureDriverShown(): Promise<boolean> {
  showTray("driver");
  if (driverViewOpen()) return true;
  await refreshDriver();
  if ((driverState?.status ?? "none") === "none") return false;
  await startDriver();
  return driverViewOpen();
}

/** Last start failure, kept until the next Start so the redraw after a failed start doesn't hide it. */
let driverError: string | null = null;

function flashDriver(e: unknown) {
  driverError = String(e).replace(/^Error: /, "");
  driverHint.hidden = false;
  driverHint.textContent = driverError;
}

async function startDriver(fresh = false) {
  driverError = null;
  if (typeof Terminal === "undefined") {
    flashDriver("the terminal component did not load");
    return;
  }
  const container = document.createElement("div");
  container.className = "term";
  driverHolder.replaceChildren(container);
  try {
    await attachDriver(container, fresh);
    showTerminal(DRIVER_ID, driverHolder);
  } catch (e) {
    await closeTerminal(DRIVER_ID);
    flashDriver(e);
  }
  await refreshDriver();
}

/** On board switch, detach the old driver (tmux keeps it running). */
export function driverBoardChanged() {
  void closeTerminal(DRIVER_ID);
  driverState = null;
  driverHolder.replaceChildren();
  if (trayTab === "driver") void refreshDriver();
}

// ---- the needs-you queue ----

export type NeedKind = "ask" | "suggest" | "stale" | "error" | "flag" | "done" | "cite" | "waiting";

export interface Need {
  kind: NeedKind;
  cell: Cell;
  text: string;
  /** When it started waiting; 0 when unknown. */
  since: number;
}

const GLYPH: Record<NeedKind | "run" | "queue" | "session", string> = {
  ask: "?",
  suggest: "◆",
  stale: "↻",
  error: "✕",
  flag: "⚑",
  done: "✓",
  cite: "\u201d",
  waiting: "❯",
  run: "●",
  queue: "○",
  session: "●",
};

/** Unsnoozed needs, oldest first. */
export function needs(): Need[] {
  return allNeeds().filter((n) => !isSnoozed(n));
}

/** The needs that want a decision; a finished answer is news, listed but not counted. */
export function needsAction(): Need[] {
  return needs().filter((n) => n.kind !== "done");
}

/** Snoozed needs, soonest to return first. */
export function snoozed(): Array<{ need: Need; until: number }> {
  return allNeeds()
    .filter(isSnoozed)
    .map((need) => ({ need, until: getLayout().snoozed![snoozeKey(need)].until }))
    .sort((a, b) => (a.until || Infinity) - (b.until || Infinity));
}

function allNeeds(): Need[] {
  const out: Need[] = [];
  for (const c of board.everywhere()) {
    if (c.decision) {
      out.push({ kind: "ask", cell: c, text: c.decision.question, since: c.decision.posed_at });
    }
    if (c.pending_suggestions.length) {
      const s = c.pending_suggestions[c.pending_suggestions.length - 1];
      const n = c.pending_suggestions.length;
      out.push({
        kind: "suggest",
        cell: c,
        text: `${n > 1 ? `${n} suggestions, latest ` : "suggests "}${firstLineOf(s.value)}${s.note ? ` · ${s.note}` : ""}`,
        since: s.at,
      });
    }
    if (c.stale_inputs?.length) {
      out.push({
        kind: "stale",
        cell: c,
        text: `session still has old values for ${c.stale_inputs.map((i) => i.name).join(", ")}`,
        since: 0,
      });
    }
    const err = errorMsg(c.state);
    if (err !== null) out.push({ kind: "error", cell: c, text: err, since: c.provenance.at });
    if (c.cite?.found === false && c.mark?.kind !== "verified") {
      out.push({ kind: "cite", cell: c, text: `quoted passage isn't in ${c.cite.file}`, since: c.cite.at });
    }
    if (c.mark?.kind === "flagged") {
      out.push({ kind: "flag", cell: c, text: c.mark.note || "flagged for a look", since: c.mark.at });
    }
    // A session at its prompt either finished or asked something in its terminal; recalc can't tell which.
    if (c.session === "waiting?") {
      out.push({ kind: "waiting", cell: c, text: "Its session is at the prompt: it finished, or it's asking you something in its terminal", since: 0 });
    }
  }
  out.push(...unseenNeeds());
  // Oldest first so new rows append and don't shift rows under the pointer.
  const at = (n: Need) => n.since || firstListed(n);
  out.sort((a, b) => at(a) - at(b));
  return out;
}

const listedAt = new Map<string, number>();
function firstListed(n: Need): number {
  const key = `${n.kind}:${n.cell.name}`;
  if (!listedAt.has(key)) listedAt.set(key, Date.now());
  return listedAt.get(key)!;
}

/** Above this many, unseen finished answers collapse into one row. */
const UNSEEN_ROWS = 3;

/** Agent answers finished since the window opened and not yet selected, excluding verified cells and the active cell. */
export function unseen(): Cell[] {
  return board
    .everywhere()
    .filter(
      (c) =>
        c.kind === "agent_call" &&
        stateKind(c.state) === "ok" &&
        c.value !== "" &&
        c.provenance.at > board.openedAt &&
        c.provenance.at > (board.seen.get(board.full(c.name)) ?? 0) &&
        c.mark?.kind !== "verified" &&
        c.name !== board.selection.active,
    )
    .sort((a, b) => a.provenance.at - b.provenance.at);
}

function unseenNeeds(): Need[] {
  const cells = unseen();
  if (cells.length <= UNSEEN_ROWS) {
    return cells.map((c) => ({ kind: "done", cell: c, text: `finished: ${firstLineOf(c.value)}`, since: c.provenance.at }));
  }
  return [{ kind: "done", cell: cells[0], text: `${cells.length} agent answers finished that you haven't looked at`, since: cells[0].provenance.at }];
}

/** Running and queued cells and running sessions. */
export function working(): Cell[] {
  return board
    .everywhere()
    .filter((c) => {
      const k = stateKind(c.state);
      // Sessions at their prompt are listed as needs instead.
      return k === "computing" || k === "queued" || c.session === "running";
    })
    .sort((a, b) => (board.workingSince.get(board.full(a.name)) ?? 0) - (board.workingSince.get(board.full(b.name)) ?? 0));
}

function snoozeKey(n: Need): string {
  return `${n.kind}:${n.cell.name}`;
}

/** A snooze ends when this changes. */
function fingerprint(n: Need): string {
  const c = n.cell;
  return `${JSON.stringify(c.state)}|${c.value}|${c.decision?.question ?? ""}|${c.pending_suggestions.length}|${c.mark?.at ?? 0}`;
}

function isSnoozed(n: Need): boolean {
  const z = getLayout().snoozed?.[snoozeKey(n)];
  if (!z) return false;
  if (z.key !== fingerprint(n)) return false;
  if (z.until && Date.now() >= z.until) return false;
  return true;
}

/** `until` 0 means until the cell changes. */
export function snooze(n: Need, until: number) {
  const snoozed = { ...(getLayout().snoozed ?? {}) };
  snoozed[snoozeKey(n)] = { until, key: fingerprint(n) } satisfies Snooze;
  setLayout({ snoozed });
  renderTray();
}

export function wake(n: Need) {
  const snoozed = { ...(getLayout().snoozed ?? {}) };
  delete snoozed[snoozeKey(n)];
  setLayout({ snoozed });
  renderTray();
}

export function wakeAll() {
  setLayout({ snoozed: {} });
  renderTray();
}

export function returnLabel(until: number, now = Date.now()): string {
  if (!until) return "until it changes";
  const d = new Date(until);
  const hm = `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
  const today = new Date(now);
  const sameDay = d.toDateString() === today.toDateString();
  const tomorrow = new Date(now + 86_400_000);
  if (sameDay) return `back ${hm}`;
  if (d.toDateString() === tomorrow.toDateString()) return `back tomorrow ${hm}`;
  return `back ${d.toLocaleDateString(undefined, { weekday: "short" })} ${hm}`;
}

let shelfOpen = false;

function renderNeedsYou(box: HTMLElement) {
  const all = needs();
  const list = all.filter((n) => n.kind !== "done");
  const news = all.filter((n) => n.kind === "done");
  const busy = working();
  if (list.length === 0 && busy.length === 0) {
    const row = document.createElement("div");
    row.className = "tray-row";
    row.textContent = "Nothing needs you right now";
    box.appendChild(row);
  }
  if (list.length) {
    box.appendChild(groupHead(`needs you · ${list.length}`));
    // Identical permission asks from several runs share one row; the answer goes to each.
    const asks = new Map<string, Need[]>();
    for (const n of list) {
      const what = n.kind === "ask" ? permissionAsk(n.cell.decision!) : null;
      if (what) asks.set(what, [...(asks.get(what) ?? []), n]);
    }
    const grouped = new Set<string>();
    for (const n of list) {
      const what = n.kind === "ask" ? permissionAsk(n.cell.decision!) : null;
      const group = what ? asks.get(what)! : [];
      if (what && group.length > 1) {
        if (!grouped.has(what)) box.appendChild(permissionGroupRow(what, group));
        grouped.add(what);
        continue;
      }
      box.appendChild(needRow(n));
    }
  }
  if (news.length) {
    const count = unseen().length;
    box.appendChild(groupHead(count === 1 ? "new answer" : `new answers · ${count}`));
    for (const n of news) box.appendChild(needRow(n));
  }
  if (busy.length) {
    box.appendChild(groupHead(`working · ${busy.length}`));
    for (const c of busy) box.appendChild(workRow(c));
  }
  const aside = snoozed();
  if (aside.length) {
    const head = groupHead(`put aside · ${aside.length}`);
    head.classList.add("shelf-head", shelfOpen ? "open" : "closed");
    head.title = shelfOpen ? "Hide what was put aside" : "Show what was put aside, soonest back first";
    head.onclick = () => {
      shelfOpen = !shelfOpen;
      renderTray();
    };
    box.appendChild(head);
    if (shelfOpen) {
      for (const { need, until } of aside) box.appendChild(shelfRow(need, until));
      const foot = document.createElement("div");
      foot.className = "tray-row tray-foot";
      const all = document.createElement("button");
      all.className = "act small";
      all.textContent = "Bring all back";
      all.onclick = wakeAll;
      foot.appendChild(all);
      box.appendChild(foot);
    }
  }
}

function shelfRow(n: Need, until: number): HTMLElement {
  const row = document.createElement("div");
  row.className = `tray-row clickable shelf need-${n.kind}`;
  row.dataset.name = n.cell.name;
  const g = document.createElement("span");
  g.className = "glyph";
  g.textContent = GLYPH[n.kind];
  const name = document.createElement("span");
  name.className = "need-cell";
  name.textContent = n.cell.name;
  const text = document.createElement("span");
  text.className = "need-text";
  text.textContent = capital(firstLineOf(n.text));
  text.title = n.text;
  const back = document.createElement("span");
  back.className = "age";
  back.dataset.until = String(until);
  back.textContent = returnLabel(until);
  const w = document.createElement("button");
  w.className = "zz";
  w.textContent = "Bring back";
  w.title = "Bring it back to the list now";
  w.onclick = (e) => {
    e.stopPropagation();
    wake(n);
  };
  row.append(g, name, text, back, w);
  return row;
}

function riskTags(what: string): HTMLElement | null {
  const risks = permissionRisks(what, board.dir ?? "");
  if (!risks.length) return null;
  const tags = document.createElement("div");
  tags.className = "risk-tags";
  for (const r of risks) {
    const t = document.createElement("span");
    t.className = "risk-tag";
    t.textContent = r;
    tags.appendChild(t);
  }
  return tags;
}

function permissionGroupRow(what: string, group: Need[]): HTMLElement {
  const row = document.createElement("div");
  row.className = "tray-row need need-ask need-group";
  row.dataset.name = group[0].cell.name;
  const g = document.createElement("span");
  g.className = "glyph";
  g.textContent = GLYPH.ask;
  g.title = "The same request from several runs";
  const head = document.createElement("div");
  head.className = "need-head";
  const name = document.createElement("span");
  name.className = "need-cell";
  const names = group.map((n) => n.cell.name);
  name.textContent = `${group.length} cells ask`;
  const age = document.createElement("span");
  age.className = "age";
  const since = Math.min(...group.map((n) => n.since || Date.now()));
  age.dataset.since = String(since);
  age.textContent = elapsedLabel(Date.now() - since);
  head.append(name, age);
  const text = document.createElement("div");
  text.className = "need-text";
  text.textContent = `The agents want to use ${what}.`;
  text.title = text.textContent;
  const cells = document.createElement("div");
  cells.className = "need-cells";
  for (const n of names) {
    const a = document.createElement("button");
    a.className = "linkish";
    a.textContent = n;
    a.title = `Select ${n}`;
    a.onclick = (e) => {
      e.stopPropagation();
      onSelect(n);
    };
    cells.appendChild(a);
  }
  const acts = document.createElement("div");
  acts.className = "need-acts";
  const tool = /^([A-Za-z]+)/.exec(what)?.[1] ?? "the tool";
  const answerAll = (label: string, title: string, index: number) => {
    const b = document.createElement("button");
    b.className = "act small";
    b.textContent = label;
    b.title = title;
    b.onclick = (e) => {
      e.stopPropagation();
      b.disabled = true;
      void Promise.all(names.map((n) => api.resolveIndex(n, index))).catch((err) => {
        b.disabled = false;
        onError(err);
      });
    };
    acts.appendChild(b);
  };
  answerAll(`Allow all ${group.length} once`, "Let each of these runs do this once", 0);
  answerAll(`Allow ${tool} for these runs`, `Don't ask these runs about ${tool} again until they end`, 1);
  answerAll(`Deny all ${group.length}`, "Each agent is told a person turned it down", 2);
  const body = document.createElement("div");
  body.className = "need-body";
  body.append(head, text);
  const tags = riskTags(what);
  if (tags) body.appendChild(tags);
  body.append(cells, acts);
  row.append(g, body);
  return row;
}

function groupHead(text: string): HTMLElement {
  const h = document.createElement("div");
  h.className = "tray-group";
  h.textContent = text;
  return h;
}

function needRow(n: Need): HTMLElement {
  const row = document.createElement("div");
  row.className = `tray-row clickable need need-${n.kind}`;
  row.dataset.name = n.cell.name;
  const g = document.createElement("span");
  g.className = "glyph";
  g.textContent = GLYPH[n.kind];
  g.title = { ask: "A question for you", suggest: "A suggested value waiting for you", stale: "Inputs this session was given have changed", error: "An error", flag: "Flagged for a look", done: "Finished since you last looked", cite: "A citation whose passage isn't in its document", waiting: "A live session sitting at its prompt" }[n.kind];
  const head = document.createElement("div");
  head.className = "need-head";
  const who = document.createElement("span");
  who.className = "asker";
  if (n.kind === "ask") {
    const d = n.cell.decision!;
    const short = actorShort(d.asker);
    // A cell's own agent reads as "A1 asks", not "A1's agent A1".
    const own = short === `${n.cell.name}'s agent`;
    const asker = short === "system" ? "you" : own ? "" : short;
    who.textContent = d.answerable_by ? `${asker || n.cell.name} → ${d.answerable_by}` : asker;
    if (d.answerable_by) who.title = `Asked of ${d.answerable_by}; you can answer it too`;
  }
  else if (n.kind === "suggest") who.textContent = actorShort(n.cell.pending_suggestions[n.cell.pending_suggestions.length - 1].from);
  const name = document.createElement("span");
  name.className = "need-cell";
  name.textContent = n.kind === "ask" && !who.textContent ? `${n.cell.name} asks` : n.cell.name;
  const age = document.createElement("span");
  age.className = "age";
  // Wait time, not time since last activity.
  if (n.since) {
    age.dataset.since = String(n.since);
    age.textContent = elapsedLabel(Date.now() - n.since);
  }
  head.append(who, name, age);
  // Questions block an agent, so they can't be snoozed.
  if (n.kind !== "ask") {
    const zz = document.createElement("button");
    zz.className = "zz";
    zz.textContent = "Later";
    zz.title = "Put this aside for an hour, until tomorrow, or until the cell changes";
    zz.onclick = (e) => {
      e.stopPropagation();
      openSnoozeMenu(zz, n);
    };
    head.appendChild(zz);
  }
  const text = document.createElement("div");
  text.className = "need-text";
  text.textContent = capital(n.text);
  text.title = n.text;
  const body = document.createElement("div");
  body.className = "need-body";
  body.append(head, text, needActions(n));
  // Show each option's preview, not only in its tooltip.
  if (n.kind === "ask") {
    const d = n.cell.decision!;
    const what = permissionAsk(d);
    const tags = what ? riskTags(what) : null;
    if (tags) body.insertBefore(tags, body.lastChild);
    // Skip raw JSON previews (a permission's request); the inspector shows those.
    const previews = d.options.map((o, i) => ({ o, i })).filter(({ o }) => o.preview && !o.preview.trimStart().startsWith("{"));
    if (previews.length) {
      const list = document.createElement("div");
      list.className = "need-previews";
      for (const { o, i } of previews) {
        const line = document.createElement("div");
        line.textContent = `${i + 1} ${o.label}: ${o.preview}`;
        list.appendChild(line);
      }
      body.appendChild(list);
    }
  }
  row.append(g, body);
  return row;
}

/** In-row actions; each makes the same engine call as the inspector. */
function needActions(n: Need): HTMLElement {
  const acts = document.createElement("div");
  acts.className = "need-acts";
  const act = (label: string, title: string, run: () => Promise<unknown>) => {
    const b = document.createElement("button");
    b.className = "act small";
    b.textContent = label;
    b.title = title;
    b.onclick = (e) => {
      e.stopPropagation();
      b.disabled = true;
      // Disabled until the call settles; on failure the row stays for a retry.
      void run().then(
        () => {
          b.disabled = false;
        },
        (err) => {
          b.disabled = false;
          onError(err);
        },
      );
    };
    acts.appendChild(b);
  };
  const c = n.cell;
  switch (n.kind) {
    case "ask": {
      const d = c.decision!;
      d.options.forEach((opt, i) => {
        act(`${i + 1} ${opt.label}`, opt.preview ?? opt.label, () => api.resolveIndex(c.name, i));
        if (i === (d.recommended ?? -1)) {
          const b = acts.lastElementChild as HTMLElement;
          b.classList.add("recommended");
          b.textContent += " (recommended)";
        }
      });
      break;
    }
    case "suggest": {
      const s = c.pending_suggestions[c.pending_suggestions.length - 1];
      act("Adopt", `Write ${firstLineOf(s.value)} into ${c.name}`, () => api.setCell(c.name, s.value));
      act("Turn down", `Keep ${c.name} as it is and drop this suggestion`, () => api.dismissSuggestion(c.name, s.value));
      break;
    }
    case "stale":
      act("Tell agent", "Type the changed inputs into its session, as you would", () => api.sessionNotify(c.name));
      break;
    case "waiting":
      act("Show", "Open its terminal in the Inspector to read what it said", async () => onShowSession(c.name));
      act("Open in Terminal", "Continue the conversation in a terminal window (⌥⌘T)", () => api.sessionHandoff(c.name, true));
      act("End session", "End the session; its conversation can still be resumed", () => api.sessionEnd(c.name));
      break;
    case "error":
      if (isRunnable(c.kind)) act("Run again", "Run the cell again", () => api.trigger(c.name));
      break;
    case "flag":
      act("Unflag", "Clear the flag", () => api.setMark([c.name], null));
      break;
    case "cite":
      act("Show source", "Open the document beside the grid to check the answer", async () => {
        onSelect(c.name);
        showTray("source");
      });
      break;
    case "done": {
      const cells = unseen();
      if (cells.length > UNSEEN_ROWS) {
        act("Review", "Check the unverified answers one at a time from the keyboard (⌥⌘R)", async () => onReview());
        act("Show next", "Select the oldest answer you haven't looked at", async () => onSelect(cells[0].name));
        act("Mark all seen", "Stop listing these answers as new", async () => {
          board.markSeen(cells.map((x) => x.name));
          renderTray();
        });
      } else {
        act("Mark seen", "Stop listing this answer as new", async () => {
          board.markSeen([c.name]);
          renderTray();
        });
      }
      break;
    }
  }
  return acts;
}

function openSnoozeMenu(anchor: HTMLElement, n: Need) {
  const menu = document.createElement("div");
  menu.className = "dropdown snooze-menu";
  const item = (label: string, until: () => number) => {
    const el = document.createElement("div");
    el.className = "item";
    el.textContent = label;
    el.onclick = (e) => {
      e.stopPropagation();
      menu.remove();
      snooze(n, until());
    };
    menu.appendChild(el);
  };
  item("for an hour", () => Date.now() + 3600_000);
  item("until tomorrow", () => {
    const d = new Date();
    d.setDate(d.getDate() + 1);
    d.setHours(9, 0, 0, 0);
    return d.getTime();
  });
  item("until it changes", () => 0);
  const r = anchor.getBoundingClientRect();
  menu.style.left = `${Math.max(0, r.right - 160)}px`;
  menu.style.top = `${r.bottom + 2}px`;
  document.body.appendChild(menu);
  const close = (e: MouseEvent) => {
    if (!menu.contains(e.target as Node)) {
      menu.remove();
      document.removeEventListener("mousedown", close);
    }
  };
  document.addEventListener("mousedown", close);
}

function workRow(c: Cell): HTMLElement {
  const row = document.createElement("div");
  row.className = "tray-row clickable work";
  row.dataset.name = c.name;
  const k = stateKind(c.state);
  const g = document.createElement("span");
  g.className = `glyph ${k === "queued" ? "g-queue" : "g-run"}`;
  g.textContent = k === "queued" ? GLYPH.queue : GLYPH.run;
  const text = document.createElement("span");
  text.className = "need-text";
  const what =
    k === "computing" ? (c.kind === "agent_call" ? (c.activity ?? "agent running") : "running") : k === "queued" ? "waiting for a free agent" : sessionWord(c.session ?? "");
  text.textContent = `${c.name}: ${what}`;
  if (c.activity) text.title = c.activity;
  const age = document.createElement("span");
  age.className = "age";
  const ms = board.workingFor(c.name);
  if (ms !== null) age.textContent = elapsedLabel(ms);
  row.append(g, text, age);
  return row;
}

/** Called once a second to update elapsed and return times in place. */
export function tickTray() {
  if (trayTab === "activity") tickFeed($("tray"));
  if (trayTab !== "decisions") return;
  // Re-render when a timed snooze expires.
  const now = Date.now();
  const z = getLayout().snoozed ?? {};
  if (Object.values(z).some((e) => e.until && e.until <= now && e.until > now - 2000)) {
    renderTray();
    return;
  }
  for (const age of document.querySelectorAll<HTMLElement>("#tray .age[data-since]")) {
    age.textContent = elapsedLabel(now - Number(age.dataset.since));
  }
  for (const back of document.querySelectorAll<HTMLElement>("#tray .age[data-until]")) {
    back.textContent = returnLabel(Number(back.dataset.until), now);
  }
  if (!document.querySelector("#tray .work")) return;
  for (const row of document.querySelectorAll<HTMLElement>("#tray .work")) {
    const ms = board.workingFor(row.dataset.name!);
    const age = row.querySelector<HTMLElement>(".age");
    if (age && ms !== null) age.textContent = elapsedLabel(ms);
  }
}

// ---- the source pane ----

function renderSource(box: HTMLElement) {
  const want = shown;
  if (!want) {
    box.replaceChildren(note("Select a cell with a citation, or any cell in a row whose column A names a document."));
    return;
  }
  const pre = box.querySelector<HTMLElement>(".source-text");
  if (pre && pre.dataset.file === want.file) {
    highlight(pre, want.quote);
    return;
  }
  let fetch = sources.get(want.file);
  if (!fetch) {
    fetch = api.readSource(want.file);
    sources.set(want.file, fetch);
  }
  box.replaceChildren(note(`opening ${want.file}…`));
  fetch.then(
    (src) => {
      if (shown?.file !== want.file || trayTab !== "source") return;
      box.replaceChildren(sourceHeader(src), sourceText(src, shown.quote));
    },
    (e) => {
      sources.delete(want.file);
      if (shown?.file !== want.file || trayTab !== "source") return;
      box.replaceChildren(note(String(e)));
    },
  );
}

function note(text: string): HTMLElement {
  const el = document.createElement("div");
  el.className = "tray-row source-note";
  el.textContent = text;
  return el;
}

function sourceHeader(src: api.Source): HTMLElement {
  const h = document.createElement("div");
  h.className = "source-head";
  const name = document.createElement("span");
  name.className = "source-file";
  name.textContent = src.file;
  name.title = src.file;
  h.appendChild(name);
  if (src.kind !== "text") {
    const k = document.createElement("span");
    k.className = "source-kind";
    k.textContent = src.kind === "pdf" ? "text of the PDF" : "text of the document";
    h.appendChild(k);
  }
  return h;
}

function sourceText(src: api.Source, quote: string | null): HTMLElement {
  const pre = document.createElement("pre");
  pre.className = "source-text";
  pre.dataset.file = src.file;
  pre.textContent = src.text;
  highlight(pre, quote);
  return pre;
}

/** Mark the quote and scroll to it. Matching ignores whitespace and quote-mark
 * differences (PDF text rarely matches exactly) and falls back to the opening words. */
export function highlight(pre: HTMLElement, quote: string | null) {
  const text = pre.textContent ?? "";
  pre.replaceChildren();
  pre.classList.remove("miss");
  const span = quote ? locate(text, quote) : null;
  if (!span) {
    pre.textContent = text;
    if (quote) pre.classList.add("miss");
    return;
  }
  const marks = paint(pre, text, span);
  queueMicrotask(() => marks[0]?.scrollIntoView?.({ block: "center" }));
}

/** Render text with [start, end) marked, one mark per line. Markdown headings
 * keep their #s (faint) so offsets match the text the quote was located in. */
function paint(pre: HTMLElement, text: string, span: [number, number] | null): HTMLElement[] {
  const marks: HTMLElement[] = [];
  const md = /\.(md|markdown)$/i.test(pre.dataset.file ?? "");
  let at = 0;
  for (const line of text.split(/(?<=\n)/)) {
    const start = at;
    const end = at + line.length;
    at = end;
    const h = md ? /^(#{1,6}) /.exec(line) : null;
    const host: HTMLElement | DocumentFragment = h ? document.createElement("span") : document.createDocumentFragment();
    if (h) (host as HTMLElement).className = `md-h md-h${h[1].length}`;
    const piece = (from: number, to: number, marked: boolean) => {
      if (from >= to) return;
      let s = text.slice(from, to);
      const target: Node = marked ? document.createElement("mark") : host;
      if (h && from === start) {
        const hash = document.createElement("span");
        hash.className = "md-hash";
        hash.textContent = h[0];
        target.appendChild(hash);
        s = s.slice(h[0].length);
      }
      target.appendChild(document.createTextNode(s));
      if (marked) {
        host.appendChild(target);
        marks.push(target as HTMLElement);
      }
    };
    if (span && span[0] < end && span[1] > start) {
      const a = Math.max(span[0], start);
      const b = Math.min(span[1], end);
      piece(start, a, false);
      piece(a, b, true);
      piece(b, end, false);
    } else piece(start, end, false);
    pre.appendChild(host);
  }
  return marks;
}

const fold = (s: string) => s.toLowerCase().replace(/[‘’]/g, "'").replace(/[“”]/g, '"');

/** The quote's [start, end) in the original text. */
export function locate(text: string, quote: string): [number, number] | null {
  // `back` maps folded offsets to original ones across collapsed whitespace.
  const folded: string[] = [];
  const back: number[] = [];
  let lastSpace = true;
  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    if (/\s/.test(ch)) {
      if (lastSpace) continue;
      lastSpace = true;
      folded.push(" ");
      back.push(i);
      continue;
    }
    lastSpace = false;
    folded.push(fold(ch));
    back.push(i);
  }
  const hay = folded.join("");
  const needleFull = fold(quote).replace(/\s+/g, " ").trim();
  const tries = [needleFull];
  const words = needleFull.split(" ");
  if (words.length > 8) tries.push(words.slice(0, 8).join(" "));
  if (words.length > 4) tries.push(words.slice(0, 4).join(" "));
  for (const needle of tries) {
    if (!needle) continue;
    const at = hay.indexOf(needle);
    if (at !== -1) return [back[at], back[at + needle.length - 1] + 1];
  }
  return null;
}

function capital(t: string): string {
  return t.charAt(0).toUpperCase() + t.slice(1);
}

