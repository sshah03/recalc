// Virtualized grid: only the visible window is in the DOM; deltas patch named cells.
// Frozen panes are four sticky layers over the scrolling body. Mouse handling is
// here; keyboard handling is in app.ts, since it depends on what has focus.

import { a1Coord, a1Name, colLetters, rect } from "./a1.js";
import { defaultAlign, formatClasses, formatValue } from "./format.js";
import { plainLine, renderProse } from "./prose.js";
import { icon } from "./icons.js";
import { sparkline } from "./spark.js";
import { board } from "./state.js";
import type { Cell } from "./types.js";
import { elapsedLabel, errorMsg, isAgent, neverRun, stateKind } from "./types.js";

/** Options that name a traffic light, for a chip's color. */
const TONES: Record<string, "good" | "warn" | "bad"> = {
  green: "good",
  yellow: "warn",
  amber: "warn",
  red: "bad",
};

const HDR_H = 28;
const HDR_W = 48;
const ROW_H = 28;
const WRAP_ROW_H = 76;
const DEFAULT_COL_W = 140;
const MIN_COL_W = 48;
/** How wide a column of document paths may fit itself before it clips. */
const MAX_FIT_W = 320;
/** An estimate of a character's width in the cell font, for fitting. */
const CHAR_W = 7;
/** Rows and columns past the data, so the sheet always has room to grow. */
const SLACK_ROWS = 60;
const SLACK_COLS = 8;
const MIN_ROWS = 100;
const MIN_COLS = 16;

/** The view state that belongs to a board and persists with it. */
export interface Layout {
  widths: Record<string, number>;
  wrap: boolean;
  frozenRows: number;
  frozenCols: number;
  /** Snoozed needs-you rows: until a time, or until the cell's fingerprint changes. */
  snoozed?: Record<string, Snooze>;
  /** Desktop notifications while in the background; on unless set false. */
  notify?: boolean;
  /** The document grid's header row and column have been auto-frozen. */
  gridFrozen?: boolean;
}

export interface Snooze {
  /** ms since epoch; 0 means "until it changes". */
  until: number;
  /** state + value at snooze time; a different one wakes the row. */
  key: string;
}

export interface GridHandlers {
  /** A click landed on a cell (or header): select it, or extend to it. */
  select(name: string, end?: string | null): void;
  /** The fill handle was dragged from the selection to `to`. */
  fillTo(to: string): void;
  /** Double-click: edit the cell in place. */
  edit(name: string): void;
  /** While a definition is being edited, clicks pick references instead. */
  pickingRefs(): boolean;
  /** Save an edit in progress; a click elsewhere on the grid ends it this way. */
  commitOpenEdit(): void;
  pickRef(anchor: string, current: string, done: boolean): void;
  contextMenu(x: number, y: number, name: string): void;
  headerMenu(x: number, y: number, axis: "rows" | "cols", index: number): void;
  layoutChanged(layout: Layout): void;
}

const $ = (id: string) => document.getElementById(id)!;

let handlers: GridHandlers;
let wrap: HTMLElement;
let sheet: HTMLElement;
let colHdr: HTMLElement;
let frozenRowsEl: HTMLElement;
let rowHdr: HTMLElement;
let frozenColsEl: HTMLElement;
let cellsEl: HTMLElement;
let layout: Layout = { widths: {}, wrap: false, frozenRows: 0, frozenCols: 0 };
/** Total sheet size in cells; grows with data and as the user scrolls out. */
let totalCols = MIN_COLS;
let totalRows = MIN_ROWS;
/** Rows shown when a filter is on, else null for all. */
let rowFilter: ((row: number) => boolean) | null = null;
let visibleRows: number[] = [];
/** Row → index among the rows below the frozen band. */
let rowIndex = new Map<number, number>();
/** Rows holding a wrapped cell whose text needs more than a line; they grow
 * to the wrap height, as the whole board does under View → Wrap Text. */
let tallRows = new Set<number>();
/** Top of each scrolling row (by its place in visibleRows) from the top of the
 * scrolling band; the last entry is the band's height. */
let rowTops: number[] = [0];
let findMatches: Set<string> | null = null;
let findCurrent: string | null = null;
const tdByName = new Map<string, HTMLElement>();
let dragSel = false;
let refDrag: { anchor: string; current: string } | null = null;
let fillDrag: string | null = null;
let frame = 0;
let hoverTimer = 0;

// ---- layout ----

export function getLayout(): Layout {
  return layout;
}

/** Replace the layout (board switch) without announcing it. */
export function loadLayout(next: Partial<Layout>) {
  layout = { widths: {}, wrap: false, frozenRows: 0, frozenCols: 0, ...next };
  renderGrid(null);
}

export function setLayout(patch: Partial<Layout>) {
  layout = { ...layout, ...patch };
  handlers.layoutChanged(layout);
  renderGrid(null);
}

export function setRowFilter(f: ((row: number) => boolean) | null) {
  rowFilter = f;
  renderGrid(null);
}

export function setFindMatches(matches: Set<string> | null, current: string | null) {
  findMatches = matches;
  findCurrent = current;
  renderGrid(null);
}

/** The usual row height (all rows, when the whole board wraps). */
function rowH(): number {
  return layout.wrap ? WRAP_ROW_H : ROW_H;
}

function rowHeight(row: number): number {
  return layout.wrap || tallRows.has(row) ? WRAP_ROW_H : ROW_H;
}

/** Top of a frozen row from the top of the frozen band. */
function frozenTop(row: number): number {
  let y = 0;
  for (let r = 1; r < row; r++) y += rowHeight(r);
  return y;
}

/** Height of the frozen rows band. */
function frozenH(): number {
  return frozenTop(layout.frozenRows + 1);
}

/** The place in visibleRows of the row at `y` in the scrolling band. */
function indexAtY(y: number): number {
  let lo = 0;
  let hi = visibleRows.length - 1;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (rowTops[mid] <= y) lo = mid;
    else hi = mid - 1;
  }
  return Math.max(0, lo);
}

export function colWidth(col: number): number {
  return layout.widths[col] ?? fitWidths.get(col) ?? DEFAULT_COL_W;
}

/** Auto-fit widths for unsized columns that hold only document paths; a clipped path hides which document a row is. */
let fitWidths = new Map<number, number>();
const PATH = /^[^\s,]+\.[A-Za-z0-9]+$/;
function fitColumns() {
  const longest = new Map<number, number>();
  const other = new Set<number>();
  for (const c of board.cells.values()) {
    const co = a1Coord(c.name);
    if (!co || co[1] === 1 || layout.widths[co[0]] !== undefined || c.value === "") continue;
    const parts = c.value.split(",").map((p) => p.trim());
    if (c.kind === "literal" && parts.every((p) => PATH.test(p))) {
      longest.set(co[0], Math.max(longest.get(co[0]) ?? 0, c.value.length));
    } else {
      other.add(co[0]);
    }
  }
  fitWidths = new Map();
  // Capped at 30% of the viewport so a frozen column A leaves room for the answers.
  const cap = wrap?.clientWidth ? Math.max(DEFAULT_COL_W, Math.floor(wrap.clientWidth * 0.3)) : MAX_FIT_W;
  for (const [col, chars] of longest) {
    if (other.has(col)) continue;
    fitWidths.set(col, Math.min(MAX_FIT_W, cap, Math.max(DEFAULT_COL_W, chars * CHAR_W + 24)));
  }
}

/** Left edge of a column relative to the first column. */
function colX(col: number): number {
  let x = 0;
  for (let c = 1; c < col; c++) x += colWidth(c);
  return x;
}

function colAtX(x: number): number {
  let acc = 0;
  for (let c = 1; c <= totalCols; c++) {
    acc += colWidth(c);
    if (x < acc) return c;
  }
  return totalCols;
}

/** Width of the frozen columns band. */
function frozenW(): number {
  return colX(layout.frozenCols + 1);
}

/** Top edge of a row measured from the top of the frozen band. */
function rowY(row: number): number | null {
  const fr = layout.frozenRows;
  if (row <= fr) return frozenTop(row);
  const idx = rowIndex.get(row);
  if (idx === undefined) return null;
  return frozenH() + rowTops[idx];
}

/** A cell's box in sheet coordinates, accounting for frozen panes; positions the editor. */
export function cellBox(name: string): { x: number; y: number; w: number; h: number } | null {
  const co = a1Coord(name);
  if (!co) return null;
  const [col, row] = co;
  const y = rowY(row);
  if (y === null) return null;
  const x = HDR_W + colX(col) + (col <= layout.frozenCols ? wrap.scrollLeft : 0);
  return { x, y: HDR_H + y + (row <= layout.frozenRows ? wrap.scrollTop : 0), w: colWidth(col), h: rowHeight(row) };
}

/** Rows a Page Up or Page Down moves: a screenful, less one for context. */
export function pageRows(): number {
  return Math.max(1, Math.floor((wrap.clientHeight - HDR_H) / ROW_H) - 1);
}

export function scrollIntoView(name: string) {
  const co = a1Coord(name);
  if (!co) return;
  const [col, row] = co;
  const w = wrap.clientWidth;
  const h = wrap.clientHeight;
  if (w === 0 || h === 0) return;
  if (col > layout.frozenCols) {
    const x0 = colX(col);
    const x1 = x0 + colWidth(col);
    const left = wrap.scrollLeft + frozenW();
    const right = wrap.scrollLeft + w - HDR_W;
    if (x0 < left) wrap.scrollLeft = x0 - frozenW();
    else if (x1 > right) wrap.scrollLeft = x1 - (w - HDR_W);
  }
  if (row > layout.frozenRows) {
    const y = rowY(row);
    if (y === null) return;
    const top = wrap.scrollTop + frozenH();
    const bottom = wrap.scrollTop + h - HDR_H;
    if (y < top) wrap.scrollTop = y - frozenH();
    else if (y + rowHeight(row) > bottom) wrap.scrollTop = y + rowHeight(row) - (h - HDR_H);
  }
}

export function initGrid(h: GridHandlers) {
  handlers = h;
  wrap = $("grid-wrap");
  sheet = $("sheet");
  colHdr = $("col-hdr");
  frozenRowsEl = $("frozen-rows");
  rowHdr = $("row-hdr");
  frozenColsEl = $("frozen-cols");
  cellsEl = $("cells");
  wrap.addEventListener("scroll", scheduleRender, { passive: true });
  window.addEventListener("resize", scheduleRender);
  wrap.addEventListener("mousedown", onMouseDown);
  wrap.addEventListener("dblclick", onDoubleClick);
  wrap.addEventListener("contextmenu", onContextMenu);
  wrap.addEventListener("mouseover", onMouseOver);
  wrap.addEventListener("mouseleave", hideHover);
  document.addEventListener("mousemove", onMouseMove);
  document.addEventListener("mouseup", onMouseUp);
}

function scheduleRender() {
  if (frame) return;
  frame = requestAnimationFrame(() => {
    frame = 0;
    renderGrid(null);
  });
}

/** Grow the sheet to cover the data, selection and viewport; compute the visible rows. */
function fitExtent() {
  const [dataCols, dataRows] = board.extent();
  let cols = Math.max(MIN_COLS, dataCols + SLACK_COLS);
  let rows = Math.max(MIN_ROWS, dataRows + SLACK_ROWS);
  const r = board.selectionRect();
  if (r) {
    cols = Math.max(cols, r[2] + SLACK_COLS);
    rows = Math.max(rows, r[3] + SLACK_ROWS);
  }
  // Always cover the viewport; scrolling to the edge keeps growing the sheet.
  if (wrap.clientHeight > 0) {
    cols = Math.max(cols, Math.ceil(wrap.clientWidth / DEFAULT_COL_W) + 2);
    rows = Math.max(rows, Math.ceil(wrap.clientHeight / rowH()) + 4);
    if (wrap.scrollTop + wrap.clientHeight >= sheet.offsetHeight - rowH()) rows = Math.max(rows, totalRows + SLACK_ROWS);
    if (wrap.scrollLeft + wrap.clientWidth >= sheet.offsetWidth - DEFAULT_COL_W) cols = Math.max(cols, totalCols + SLACK_COLS);
  }
  totalCols = Math.max(totalCols, cols);
  totalRows = Math.max(totalRows, rows);

  const fr = layout.frozenRows;
  rowIndex = new Map();
  visibleRows = [];
  if (rowFilter) {
    // A filter shows matching data rows only; the sheet ends with the data.
    for (let row = fr + 1; row <= dataRows; row++) {
      if (rowFilter(row)) visibleRows.push(row);
    }
  } else {
    for (let row = fr + 1; row <= totalRows; row++) visibleRows.push(row);
  }
  visibleRows.forEach((row, i) => rowIndex.set(row, i));

  tallRows = new Set();
  for (const c of board.cells.values()) {
    if (!c.format?.wrap || !c.value) continue;
    const co = a1Coord(c.name);
    if (co && (c.value.includes("\n") || c.value.length * CHAR_W > colWidth(co[0]) - 16)) tallRows.add(co[1]);
  }
  rowTops = [0];
  for (const row of visibleRows) rowTops.push(rowTops[rowTops.length - 1] + rowHeight(row));
}

/** Redraw. `null` rebuilds the visible window; a set of names patches only those cells, if in view. */
export function renderGrid(changed: Set<string> | null) {
  // A wrapped cell's text can change its row's height: lay the window out again.
  if (changed && [...changed].some((n) => board.get(n)?.format?.wrap)) changed = null;
  if (changed && changed.size > 0) {
    for (const name of changed) {
      const td = tdByName.get(name);
      if (td) paintCell(td, name);
    }
    refreshColumnBadges();
    return;
  }
  fitColumns();
  fitExtent();
  tallyColumns();
  const fr = layout.frozenRows;
  const fc = layout.frozenCols;
  const fw = frozenW();
  const fh = frozenH();
  const totalW = HDR_W + colX(totalCols + 1);
  const scrollH = rowTops[visibleRows.length];
  const bodyH = fh + scrollH;
  sheet.style.width = `${totalW}px`;
  sheet.style.height = `${HDR_H + bodyH}px`;
  wrap.setAttribute("aria-rowcount", String(totalRows + 1));
  wrap.setAttribute("aria-colcount", String(totalCols + 1));
  colHdr.style.width = `${totalW}px`;
  frozenRowsEl.style.width = `${totalW}px`;
  frozenRowsEl.style.height = `${fh}px`;
  frozenRowsEl.style.top = `${HDR_H}px`;
  frozenRowsEl.hidden = fr === 0;
  rowHdr.style.height = `${scrollH}px`;
  frozenColsEl.style.width = `${fw}px`;
  frozenColsEl.style.height = `${scrollH}px`;
  frozenColsEl.style.left = `${HDR_W}px`;
  frozenColsEl.hidden = fc === 0;
  cellsEl.style.width = `${totalW - HDR_W - fw}px`;
  cellsEl.style.height = `${scrollH}px`;
  document.body.classList.toggle("wrap", layout.wrap);

  // Visible window plus a margin, or a fixed block before layout (first paint, jsdom).
  const vw = wrap.clientWidth || 1400;
  const vh = wrap.clientHeight || 800;
  const i0 = Math.max(0, indexAtY(wrap.scrollTop) - 2);
  const i1 = Math.min(visibleRows.length - 1, indexAtY(wrap.scrollTop + vh) + 2);
  const c0 = Math.max(fc + 1, colAtX(Math.max(0, wrap.scrollLeft + fw)) - 1);
  const c1 = Math.min(totalCols, colAtX(wrap.scrollLeft + vw) + 1);
  const selRect = board.selectionRect();
  const frozenColList = Array.from({ length: fc }, (_, i) => i + 1);
  const scrollCols = Array.from({ length: Math.max(0, c1 - c0 + 1) }, (_, i) => c0 + i);
  const frozenRowList = Array.from({ length: fr }, (_, i) => i + 1);
  const scrollRows = visibleRows.slice(i0, i1 + 1);

  tdByName.clear();

  // Column headers: the corner, a sticky block for frozen columns, the rest.
  colHdr.replaceChildren();
  const corner = document.createElement("div");
  corner.id = "corner";
  colHdr.appendChild(corner);
  const hdrFrozen = document.createElement("div");
  hdrFrozen.className = "frozen-band";
  hdrFrozen.style.left = `${HDR_W}px`;
  hdrFrozen.style.width = `${fw}px`;
  for (const col of frozenColList) hdrFrozen.appendChild(colHeader(col, colX(col), selRect));
  colHdr.appendChild(hdrFrozen);
  for (const col of scrollCols) colHdr.appendChild(colHeader(col, HDR_W + colX(col), selRect));

  // Frozen rows band: row headers (sticky), frozen corner cells (sticky), the rest.
  frozenRowsEl.replaceChildren();
  if (fr > 0) {
    const hdrs = document.createElement("div");
    hdrs.className = "frozen-band rowhdrs";
    hdrs.style.left = "0px";
    hdrs.style.width = `${HDR_W}px`;
    for (const row of frozenRowList) hdrs.appendChild(rowHeader(row, frozenTop(row), selRect));
    frozenRowsEl.appendChild(hdrs);
    const cornerCells = document.createElement("div");
    cornerCells.className = "frozen-band";
    cornerCells.style.left = `${HDR_W}px`;
    cornerCells.style.width = `${fw}px`;
    for (const row of frozenRowList) for (const col of frozenColList) cornerCells.appendChild(cellEl(col, row, colX(col), frozenTop(row)));
    frozenRowsEl.appendChild(cornerCells);
    for (const row of frozenRowList) for (const col of scrollCols) frozenRowsEl.appendChild(cellEl(col, row, HDR_W + colX(col), frozenTop(row)));
  }

  // Scrolling body: row headers, frozen columns, cells.
  rowHdr.replaceChildren();
  frozenColsEl.replaceChildren();
  cellsEl.replaceChildren();
  for (const row of scrollRows) {
    const y = rowTops[rowIndex.get(row)!];
    rowHdr.appendChild(rowHeader(row, y, selRect));
    for (const col of frozenColList) frozenColsEl.appendChild(cellEl(col, row, colX(col), y));
    for (const col of scrollCols) cellsEl.appendChild(cellEl(col, row, colX(col) - fw, y));
  }
  placeFillHandle(selRect);
}

/** Running or queued cells per column, for the headers. Computed once per render. */
let columnBusy = new Map<number, number>();
function tallyColumns() {
  columnBusy = new Map();
  for (const c of board.cells.values()) {
    const k = stateKind(c.state);
    if (k !== "computing" && k !== "queued") continue;
    const co = a1Coord(c.name);
    if (co) columnBusy.set(co[0], (columnBusy.get(co[0]) ?? 0) + 1);
  }
}

/** A delta changed some states: fix the counts on the headers in place. */
function refreshColumnBadges() {
  tallyColumns();
  for (const th of colHdr.querySelectorAll<HTMLElement>(".hdr.col")) {
    const col = Number(th.dataset.col);
    const busy = columnBusy.get(col);
    const have = th.querySelector<HTMLElement>(".col-busy");
    if (!busy) {
      have?.remove();
    } else if (have) {
      have.textContent = String(busy);
    } else {
      th.appendChild(busyBadge(busy));
    }
  }
}

function busyBadge(n: number): HTMLElement {
  const b = document.createElement("span");
  b.className = "col-busy";
  b.textContent = String(n);
  b.title = `${n} running or queued in this column`;
  return b;
}

function colHeader(col: number, x: number, selRect: [number, number, number, number] | null): HTMLElement {
  const th = document.createElement("div");
  th.className = "hdr col";
  th.setAttribute("role", "columnheader");
  th.setAttribute("aria-colindex", String(col + 1));
  th.dataset.col = String(col);
  th.style.left = `${x}px`;
  th.style.width = `${colWidth(col)}px`;
  th.textContent = colLetters(col);
  const busy = columnBusy.get(col);
  if (busy) th.appendChild(busyBadge(busy));
  if (selRect && col >= selRect[0] && col <= selRect[2]) th.classList.add("active");
  th.title = "Click to select the column, drag the edge to resize, right-click for more";
  const grip = document.createElement("div");
  grip.className = "resize";
  th.appendChild(grip);
  return th;
}

function rowHeader(row: number, y: number, selRect: [number, number, number, number] | null): HTMLElement {
  const rhd = document.createElement("div");
  rhd.className = "hdr row";
  rhd.setAttribute("role", "rowheader");
  rhd.setAttribute("aria-rowindex", String(row + 1));
  rhd.dataset.row = String(row);
  rhd.style.top = `${y}px`;
  rhd.style.height = `${rowHeight(row)}px`;
  rhd.style.lineHeight = `${rowHeight(row)}px`;
  rhd.textContent = String(row);
  if (selRect && row >= selRect[1] && row <= selRect[3]) rhd.classList.add("active");
  return rhd;
}

function cellEl(col: number, row: number, x: number, y: number): HTMLElement {
  const name = a1Name(col, row);
  const td = document.createElement("div");
  td.dataset.name = name;
  td.setAttribute("role", "gridcell");
  td.setAttribute("aria-rowindex", String(row + 1));
  td.setAttribute("aria-colindex", String(col + 1));
  td.style.left = `${x}px`;
  td.style.top = `${y}px`;
  td.style.width = `${colWidth(col)}px`;
  td.style.height = `${rowHeight(row)}px`;
  tdByName.set(name, td);
  paintCell(td, name);
  return td;
}

/** The fill handle at the selection's bottom-right corner. */
function placeFillHandle(selRect: [number, number, number, number] | null) {
  const h = $("fill-handle");
  const box = selRect && !refDrag ? cellBox(a1Name(selRect[2], selRect[3])) : null;
  if (!box) {
    h.hidden = true;
    return;
  }
  h.style.left = `${box.x + box.w - 5}px`;
  h.style.top = `${box.y + box.h - 5}px`;
  h.hidden = false;
}

/** Draw one cell with its selection, dependency and find highlights. Cheap enough per delta. */
function paintCell(td: HTMLElement, name: string) {
  td.replaceChildren();
  const cell = board.get(name);
  const sel = board.selection;
  const selRect = board.selectionRect();
  const co = a1Coord(name)!;
  const inside = (r: [number, number, number, number] | null) =>
    !!r && co[0] >= r[0] && co[0] <= r[2] && co[1] >= r[1] && co[1] <= r[3];
  td.className = "cell";
  if (name === sel.active && !refDrag) td.classList.add("selected");
  if (inside(selRect) && (sel.end || refDrag)) td.classList.add("inrange");
  td.setAttribute("aria-selected", String(name === sel.active || (inside(selRect) && !!sel.end)));
  if (refDrag && inside(rect(refDrag.anchor, refDrag.current))) td.classList.add("picking");
  if (fillDrag && selRect && inside(fillTarget(selRect, fillDrag))) td.classList.add("filling");
  if (findMatches?.has(name)) td.classList.add(name === findCurrent ? "match-current" : "match");
  // Precedents of the active cell wear its outline color; dependents dim.
  const active = board.active;
  if (active && active.name !== name) {
    if (active.deps.includes(name)) td.classList.add("dep");
    else if (cell?.deps.includes(active.name)) td.classList.add("dependent");
  }
  if (!cell) return;
  cellFace(td, cell);
  // The selected cell with a set of values (options, Yes or No, a date) opens them from ▾.
  const kind = cell?.answer_type?.kind;
  if (name === sel.active && !refDrag && (kind === "choice" || kind === "yesno" || kind === "date")) {
    const b = document.createElement("span");
    b.className = "pick-btn";
    b.title = kind === "date" ? "Choose a date" : "Choose a value";
    b.appendChild(icon("chevron", 12));
    td.appendChild(b);
  }
  // Long row-1 headers take two smaller lines within the same row height; uniform
  // row heights keep the virtual window arithmetic.
  if (co[1] === 1 && cell.kind === "literal" && !layout.wrap && firstLine(cell.value).length * CHAR_W > colWidth(co[0]) - 16) {
    td.classList.add("head2");
  }
}

export function valueClass(cell: Cell): string {
  const k = stateKind(cell.state);
  if (k === "computing") return "v-computing";
  if (k === "queued") return "v-queued";
  if (k === "stale") return "v-stale";
  if (k === "error") return "v-error";
  if (k === "quarantined") return "v-quarantined";
  if (k === "awaiting") return "v-awaiting";
  if (cell.value === "true") return asksAboutAProblem(cell) ? "v-false" : "v-true";
  if (cell.value === "false") return asksAboutAProblem(cell) ? "v-true" : "v-false";
  return "";
}

/** A check named for a problem (`regressed?`, `tests_failing`) is good when false, so its colors flip.
 * Matched against the cell name or its column header. */
const PROBLEM = /regress|fail|broken|error|contradict|mismatch|stale|missing|leak|violat|blocked|exceed|over_budget|drift|flaky|unsafe|vulnerab|exploit/i;
function asksAboutAProblem(cell: Cell): boolean {
  if (PROBLEM.test(cell.name)) return true;
  const co = a1Coord(cell.name);
  return co !== null && co[1] > 1 && PROBLEM.test(board.get(a1Name(co[0], 1))?.value ?? "");
}

/** A face shows at most this much; laying out a huge value on every repaint stalls the window. */
const FACE_MAX = 2000;
function faceText(v: string): string {
  return v.length > FACE_MAX ? `${v.slice(0, FACE_MAX)}…` : v;
}

export function firstLine(v: string): string {
  const i = v.indexOf("\n");
  return i === -1 ? v : v.slice(0, i);
}

/** Placeholder text for an empty face whose state would otherwise be invisible. */
function stateFallback(k: string): string {
  return k === "computing" ? "running…" : k === "queued" ? "queued" : k === "error" ? "error" : "";
}

function badge(text: string, cls: string): HTMLElement {
  const b = document.createElement("span");
  b.className = `badge ${cls}`;
  b.textContent = text;
  return b;
}

/** The cell's face: marks, badges, value, sparkline, provenance. Shared with the named-cells panel. */
export function cellFace(td: HTMLElement, cell: Cell) {
  if (board.pulsed.has(cell.name)) {
    td.classList.remove("pulse");
    void td.offsetWidth; // restart the animation
    td.classList.add("pulse");
  }
  const k = stateKind(cell.state);
  if (cell.mark) td.classList.add(`mark-${cell.mark.kind}`);
  if (cell.cite) td.classList.add("cited");
  for (const c of formatClasses(cell.format, cell.value)) td.classList.add(c);
  td.classList.add(`al-${defaultAlign(cell.format)}`);
  if (k === "awaiting") td.appendChild(badge("?", "throb"));
  if (cell.session) {
    td.appendChild(badge(cell.session, cell.session === "waiting?" ? "throb" : "running"));
  }
  if (cell.stale_inputs?.length) td.appendChild(badge("inputs changed", "stale-inputs"));
  if (k === "computing") {
    const spin = document.createElement("span");
    spin.className = "spinner";
    spin.title = "Running";
    td.appendChild(spin);
  }
  if (cell.mark) {
    // A shape as well as a tint, so the mark doesn't depend on color alone.
    const m = document.createElement("span");
    m.className = `mark-glyph ${cell.mark.kind}`;
    m.textContent = cell.mark.kind === "verified" ? "\u2713" : "\u2691";
    m.title = cell.mark.kind === "verified" ? "Verified" : cell.mark.note ? `Flagged: ${cell.mark.note}` : "Flagged";
    td.appendChild(m);
  }
  const val = document.createElement("span");
  val.className = `val ${valueClass(cell)}`;
  // Wrapped (the whole board, or this cell's format): every line, not the first.
  const wraps = layout.wrap || !!cell.format?.wrap;
  const raw = faceText(wraps ? cell.value : firstLine(cell.value));
  const shown = formatValue(cell.kind === "agent_call" && !wraps ? plainLine(raw) : raw, cell.format?.num, {
    decimals: cell.format?.decimals,
    currency: cell.format?.currency,
  });
  // The error shows on the face only when there is no last value; otherwise in the hover card.
  const err = errorMsg(cell.state);
  val.textContent = shown || (err !== null ? `error: ${firstLine(err)}` : stateFallback(k));
  // The stylesheet labels never-run agents; the value stays empty.
  if (!val.textContent && neverRun(cell)) {
    td.classList.add("unrun");
  }
  // Choice answers render as chips, traffic-light options in their colors.
  const opts = cell.answer_type?.kind === "choice" ? (cell.answer_type.options ?? []) : [];
  if (cell.value && opts.some((o) => o.toLowerCase() === cell.value.trim().toLowerCase())) {
    val.classList.add("chip");
    const tone = TONES[cell.value.trim().toLowerCase()];
    if (tone) val.classList.add(`tone-${tone}`);
  }
  td.appendChild(val);
  if (k === "computing" || k === "queued") {
    const el = document.createElement("span");
    el.className = "elapsed";
    const ms = board.workingFor(cell.name);
    if (ms !== null) el.textContent = elapsedLabel(ms);
    td.appendChild(el);
  }
  if (cell.cite) {
    // Closing quote mark: the value has a citation.
    const q = document.createElement("span");
    const missing = cell.cite.found === false;
    q.className = missing ? "cite-mark missing" : "cite-mark";
    // Struck quote mark for a missing passage, so it doesn't rely on color.
    q.textContent = missing ? "\u201d\u0337" : "\u201d";
    q.title = missing ? `The cited passage isn't in ${cell.cite.file} as quoted` : `Has a citation: ${cell.cite.file}`;
    td.appendChild(q);
  }
  if (cell.series && cell.spark) {
    const s = sparkline(cell.spark);
    if (s) td.appendChild(s);
  }
  // Corner mark for a value written by another cell's agent. Skipped on empty cells,
  // on self-writes, and on a table's header row and column A, which a driver writes wholesale.
  const writer = cell.provenance.by.slice(6);
  const co = a1Coord(cell.name);
  const frame = co !== null && (co[1] === 1 || co[0] === 1);
  if (isAgent(cell.provenance.by) && cell.value !== "" && writer !== cell.name && !frame) {
    const p = document.createElement("span");
    p.className = "prov";
    p.title = `Written by ${writer}`;
    td.appendChild(p);
  }
}

/** Once a second: the elapsed labels on working cells, written in place. */
export function tickElapsed() {
  for (const el of document.querySelectorAll<HTMLElement>("#cells .cell .elapsed, #named .elapsed")) {
    const name = el.closest<HTMLElement>("[data-name]")?.dataset.name;
    const ms = name ? board.workingFor(name) : null;
    if (ms !== null) el.textContent = elapsedLabel(ms);
  }
}

// ---- hover card: the full value of a cell that doesn't fit ----

function onMouseOver(e: MouseEvent) {
  if (cardPinned()) return; // an expanded card stays until the selection moves
  const td = (e.target as HTMLElement).closest<HTMLElement>(".cell");
  window.clearTimeout(hoverTimer);
  const cell = td ? board.get(td.dataset.name!) : undefined;
  if (!td || !cell || dragSel || refDrag || fillDrag) {
    hideHover();
    return;
  }
  const val = td.querySelector<HTMLElement>(".val");
  const overflows =
    (val && (val.scrollWidth > val.clientWidth + 1 || val.scrollHeight > val.clientHeight + 1)) || cell.value.includes("\n") || cell.value.length > 60;
  const err = errorMsg(cell.state);
  const working = board.workingFor(cell.name);
  if (!overflows && err === null && working === null && !cell.mark?.note) {
    hideHover();
    return;
  }
  hoverTimer = window.setTimeout(() => {
    const card = $("hovercard");
    const lines: string[] = [];
    if (err !== null) lines.push(`error: ${err}`);
    if (working !== null) lines.push(`${stateKind(cell.state) === "queued" ? "queued" : "running"} for ${elapsedLabel(working)}`);
    if (cell.mark?.kind === "flagged") lines.push(cell.mark.note ? `flagged: ${cell.mark.note}` : "flagged");
    fillCard(card, lines, cell);
    const r = td.getBoundingClientRect();
    card.hidden = false;
    card.style.left = `${Math.max(0, Math.min(r.left, window.innerWidth - 440))}px`;
    card.style.top = `${r.bottom + 4}px`;
  }, 350);
}

/** The card's text: notes first, then the value, an agent's as prose. */
function fillCard(card: HTMLElement, notes: string[], cell: Cell) {
  card.replaceChildren();
  card.classList.toggle("prose-card", cell.kind === "agent_call");
  if (notes.length) {
    const n = document.createElement("div");
    n.className = "card-notes";
    n.textContent = notes.join("\n");
    card.appendChild(n);
  }
  if (!cell.value) return;
  const v = document.createElement("div");
  // Truncated: laying out a whole log on hover stalls the window.
  const text = cell.value.length > 4000 ? `${cell.value.slice(0, 4000)}…` : cell.value;
  if (cell.kind === "agent_call") renderProse(v, text);
  else v.textContent = text;
  card.appendChild(v);
}

/** Space on a filled cell: show its whole value in a card until the selection moves or Esc. */
export function expandCell(name: string): boolean {
  const cell = board.get(name);
  const td = tdByName.get(name);
  if (!cell || !cell.value || !td) return false;
  const card = $("hovercard");
  const notes: string[] = [];
  if (cell.mark?.kind === "flagged") notes.push(cell.mark.note ? `flagged: ${cell.mark.note}` : "flagged");
  if (cell.cite) notes.push(`cites ${cell.cite.file}${cell.cite.found === false ? " (the quote isn't in the file as written)" : ""}`);
  fillCard(card, notes, cell);
  card.classList.add("pinned");
  const r = td.getBoundingClientRect();
  card.hidden = false;
  card.style.left = `${Math.max(0, Math.min(r.left, window.innerWidth - 560))}px`;
  card.style.top = `${Math.min(r.bottom + 4, window.innerHeight - 200)}px`;
  return true;
}

export function cardPinned(): boolean {
  const card = document.getElementById("hovercard");
  return !!card && !card.hidden && card.classList.contains("pinned");
}

function hideHover() {
  window.clearTimeout(hoverTimer);
  const card = document.getElementById("hovercard");
  if (card) {
    card.hidden = true;
    card.classList.remove("pinned");
  }
}

/** Close an expanded cell's card. */
export function closeCard() {
  if (cardPinned()) hideHover();
}

// ---- mouse ----

function cellAt(target: EventTarget | null): string | null {
  return (target as HTMLElement | null)?.closest<HTMLElement>(".cell")?.dataset.name ?? null;
}

/** Fill target: `sel` extended along the axis the pointer moved furthest, never both. */
/** Where a double click on the fill handle fills to: the last row of the run
 * of values beside the selection (left first, then right), or nowhere. */
export function autoFillTarget(): string | null {
  const sel = board.selectionRect();
  if (!sel) return null;
  const [c0, , c1, r1] = sel;
  const filled = (col: number, row: number) => !!board.get(a1Name(col, row))?.value.trim();
  for (const col of [c0 - 1, c1 + 1]) {
    if (col < 1 || !filled(col, r1 + 1)) continue;
    let row = r1 + 1;
    while (filled(col, row + 1)) row++;
    return a1Name(c1, row);
  }
  return null;
}

export function fillTarget(sel: [number, number, number, number], to: string): [number, number, number, number] | null {
  const co = a1Coord(to);
  if (!co) return null;
  const down = co[1] - sel[3];
  const right = co[0] - sel[2];
  if (down <= 0 && right <= 0) return null;
  return down >= right ? [sel[0], sel[1], sel[2], co[1]] : [sel[0], sel[1], co[0], sel[3]];
}

function onMouseDown(e: MouseEvent) {
  if (e.button !== 0) return;
  hideHover();
  const t = e.target as HTMLElement;
  if (t.id === "fill-handle") {
    e.preventDefault();
    // A double click fills down as far as the column beside has data, as in
    // Sheets and Excel.
    if (e.detail === 2) {
      fillDrag = null;
      const to = autoFillTarget();
      if (to) handlers.fillTo(to);
      return;
    }
    fillDrag = board.selection.active;
    return;
  }
  const grip = t.closest<HTMLElement>(".resize");
  if (grip) {
    startColResize(e, grip);
    return;
  }
  const name = cellAt(t);
  if (name && t.closest(".pick-btn")) {
    // Keep focus for the editor the list opens in.
    e.preventDefault();
    handlers.edit(name);
    return;
  }
  if (name) {
    if (handlers.pickingRefs()) {
      // Keep the editor focused; the click picks a reference.
      e.preventDefault();
      refDrag = { anchor: name, current: name };
      handlers.pickRef(name, name, false);
      renderGrid(null);
      return;
    }
    // The second press of a double click edits, as in Sheets and Excel. The
    // first press re-renders the cell, so WebKit never fires click or
    // dblclick: the press's own count is what's left to go on.
    if (e.detail === 2 && !e.shiftKey && board.selection.active === name && !board.selection.end) {
      e.preventDefault();
      handlers.edit(name);
      return;
    }
    handlers.commitOpenEdit();
    if (e.shiftKey && board.selection.active && a1Coord(board.selection.active)) {
      handlers.select(board.selection.active, name);
    } else {
      handlers.select(name);
      dragSel = true;
    }
    return;
  }
  const col = t.closest<HTMLElement>(".hdr.col")?.dataset.col;
  if (col) {
    handlers.commitOpenEdit();
    const c = Number(col);
    const lastRow = Math.max(1, board.extent()[1]);
    const a = e.shiftKey && board.selection.active ? a1Coord(board.selection.active) : null;
    if (a) handlers.select(a1Name(a[0], 1), a1Name(c, lastRow));
    else handlers.select(a1Name(c, 1), a1Name(c, lastRow));
    return;
  }
  const row = t.closest<HTMLElement>(".hdr.row")?.dataset.row;
  if (row) {
    handlers.commitOpenEdit();
    const r = Number(row);
    const lastCol = Math.max(1, board.extent()[0]);
    const a = e.shiftKey && board.selection.active ? a1Coord(board.selection.active) : null;
    if (a) handlers.select(a1Name(1, a[1]), a1Name(lastCol, r));
    else handlers.select(a1Name(1, r), a1Name(lastCol, r));
  }
}

function onMouseMove(e: MouseEvent) {
  if (!dragSel && !refDrag && !fillDrag) return;
  const name = cellAt(e.target);
  if (!name) return;
  if (fillDrag) {
    if (fillDrag !== name) {
      fillDrag = name;
      renderGrid(null);
    }
    return;
  }
  if (refDrag) {
    if (refDrag.current !== name) {
      refDrag.current = name;
      handlers.pickRef(refDrag.anchor, name, false);
      renderGrid(null);
    }
  } else if (board.selection.active) {
    const end = name === board.selection.active ? null : name;
    if (end !== board.selection.end) handlers.select(board.selection.active, end);
  }
}

function onMouseUp() {
  if (fillDrag) {
    const sel = board.selectionRect();
    const target = sel ? fillTarget(sel, fillDrag) : null;
    fillDrag = null;
    if (target) handlers.fillTo(a1Name(target[2], target[3]));
    else renderGrid(null);
    return;
  }
  if (refDrag) {
    handlers.pickRef(refDrag.anchor, refDrag.current, true);
    refDrag = null;
    renderGrid(null);
  }
  dragSel = false;
}

function onDoubleClick(e: MouseEvent) {
  const name = cellAt(e.target);
  if (name && !handlers.pickingRefs()) handlers.edit(name);
}

function onContextMenu(e: MouseEvent) {
  const t = e.target as HTMLElement;
  const col = t.closest<HTMLElement>(".hdr.col")?.dataset.col;
  const row = t.closest<HTMLElement>(".hdr.row")?.dataset.row;
  if (col || row) {
    e.preventDefault();
    handlers.headerMenu(e.clientX, e.clientY, col ? "cols" : "rows", Number(col ?? row));
    return;
  }
  const name = cellAt(t);
  if (!name) return;
  e.preventDefault();
  const r = board.selectionRect();
  const co = a1Coord(name)!;
  const inside = r && co[0] >= r[0] && co[0] <= r[2] && co[1] >= r[1] && co[1] <= r[3];
  if (!inside) handlers.select(name);
  handlers.contextMenu(e.clientX, e.clientY, name);
}

function startColResize(e: MouseEvent, grip: HTMLElement) {
  e.preventDefault();
  const col = Number(grip.parentElement!.dataset.col);
  const startX = e.clientX;
  const startW = colWidth(col);
  const onMove = (ev: MouseEvent) => {
    layout = { ...layout, widths: { ...layout.widths, [col]: Math.max(MIN_COL_W, startW + ev.clientX - startX) } };
    renderGrid(null);
  };
  window.addEventListener("mousemove", onMove);
  window.addEventListener(
    "mouseup",
    () => {
      window.removeEventListener("mousemove", onMove);
      handlers.layoutChanged(layout);
    },
    { once: true },
  );
}
