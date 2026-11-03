// The board as the UI knows it: a map of cells kept current by snapshot
// deltas, the selection, and a change signal the renderers subscribe to.
// Nothing here talks to the backend; app.ts feeds snapshots in.

import { a1Coord, rect, rectNames } from "./a1.js";
import type { ActivityEntry, AgentLoad, Cell, Snapshot } from "./types.js";

export interface Selection {
  /** The active cell (A1 or named); null when nothing is selected. */
  active: string | null;
  /** The far corner of a range selection, when the active cell is A1. */
  end: string | null;
}

export type Listener = (changed: Set<string> | null) => void;

/** The board's sheets: the first (its cells unprefixed) and the others, in tab order. */
export interface Sheets {
  first: string;
  others: string[];
}

/** A session's status cell, as the backend names it (`status_name`). */
export function statusName(cell: string): string {
  const flat = cell.replace(/[^A-Za-z0-9_]/g, "_");
  return `${/^[0-9]/.test(flat) ? "_" : ""}${flat}_status`;
}

/** A sheet as a reference writes it: quoted unless it's a plain word. */
export function quoteSheet(sheet: string): string {
  return /^[A-Za-z_][A-Za-z0-9_]*$/.test(sheet) && !a1Coord(sheet) ? sheet : `'${sheet.replace(/'/g, "''")}'`;
}

class Board {
  dir = "";
  sheets: Sheets = { first: "Sheet1", others: [] };
  /** The sheet the grid shows; null for the first. */
  sheet: string | null = null;
  /** Every cell on every sheet, by its full name (`Notes!A2`). */
  all = new Map<string, Cell>();
  /** What the grid works with: the shown sheet's cells by their local
   * names (`A2`) and the workbook-wide named cells. Cells elsewhere are
   * reached with `get`, by full names (`Notes!A2`, `Sheet1!A2`). */
  cells = new Map<string, Cell>();
  activity: ActivityEntry[] = [];
  agents: AgentLoad = { running: 0, queued: 0, max: 1 };
  /** Cells whose value differed from the previous snapshot: the pulse. */
  pulsed = new Set<string>();
  /** When each working cell was first seen computing or queued (ms since
   * epoch), for the elapsed timers. Cleared when it lands. */
  workingSince = new Map<string, number>();
  selection: Selection = { active: null, end: null };
  /** When this window opened; answers landing later are news until their cell is
   * selected. Per window, not on disk. */
  readonly openedAt = Date.now();
  /** Per cell, the write time (provenance.at) the person last selected it at. */
  seen = new Map<string, number>();
  private listeners: Listener[] = [];

  onChange(fn: Listener) {
    this.listeners.push(fn);
  }

  /** Apply a snapshot; `null` in the signal means "everything may differ". */
  apply(snap: Snapshot) {
    const boardChanged = snap.dir !== this.dir;
    this.dir = snap.dir;
    this.activity = snap.activity;
    this.agents = snap.agents;
    if (snap.sheets) this.sheets = snap.sheets;
    if (boardChanged || (this.sheet !== null && !this.sheets.others.includes(this.sheet))) this.sheet = null;
    this.pulsed = new Set();
    const changed = new Set<string>();
    if (snap.full) {
      const prev = this.all;
      this.all = new Map();
      for (const c of snap.cells) {
        this.all.set(c.name, c);
        if (prev.get(c.name)?.value !== c.value && prev.has(c.name)) this.pulsed.add(this.local(c.name));
        this.trackWork(c);
      }
      for (const name of [...this.workingSince.keys()]) if (!this.all.has(name)) this.workingSince.delete(name);
      if (boardChanged) this.selection = { active: null, end: null };
      this.rebuildView();
      this.emit(null);
      return;
    }
    for (const c of snap.cells) {
      const prev = this.all.get(c.name);
      if (prev && prev.value !== c.value) this.pulsed.add(this.local(c.name));
      this.all.set(c.name, c);
      this.trackWork(c);
      this.viewOne(c.name);
      changed.add(this.local(c.name));
      // A session that ended writes only its status cell; the agent cell
      // isn't sent again, so drop the live status it still carries here.
      const owner = [...this.all.values()].find((o) => o.session && statusName(o.name) === c.name);
      if (owner && !/^(running|waiting)/.test(c.value)) {
        this.all.set(owner.name, { ...owner, session: undefined });
        this.viewOne(owner.name);
        changed.add(this.local(owner.name));
      }
    }
    for (const name of snap.removed) {
      this.all.delete(name);
      this.cells.delete(this.local(name));
      this.workingSince.delete(name);
      changed.add(this.local(name));
    }
    this.emit(changed);
  }

  /** Show sheet `sheet` (null: the first) in the grid. */
  setSheet(sheet: string | null) {
    if (sheet === this.sheets.first) sheet = null;
    if (sheet === this.sheet) return;
    this.sheet = sheet;
    this.selection = { active: null, end: null };
    this.rebuildView();
    this.emit(null);
  }

  /** The shown sheet's name, as its tab reads. */
  get sheetName(): string {
    return this.sheet ?? this.sheets.first;
  }

  /** A full name as the grid names it: the shown sheet's cells by their
   * local names, the first sheet's (seen from another) as `Sheet1!A2`. */
  local(full: string): string {
    const bang = full.lastIndexOf("!");
    if (bang >= 0) return full.slice(0, bang) === this.sheet ? full.slice(bang + 1) : full;
    if (this.sheet !== null && a1Coord(full)) return `${this.sheets.first}!${full}`;
    return full;
  }

  /** The inverse of `local`: what the backend calls the cell. */
  full(name: string): string {
    const bang = name.lastIndexOf("!");
    if (bang >= 0) {
      const sheet = name.slice(0, bang).replace(/^'(.*)'$/, "$1").replace(/''/g, "'");
      const local = name.slice(bang + 1);
      return sheet.toLowerCase() === this.sheets.first.toLowerCase() ? local : `${sheet}!${local}`;
    }
    return this.sheet !== null && a1Coord(name) ? `${this.sheet}!${name}` : name;
  }

  /** `c` as the grid sees it: its name and deps local. */
  private viewed(c: Cell): Cell {
    if (this.sheet === null && !c.name.includes("!") && !c.deps.some((d) => d.includes("!"))) return c;
    return { ...c, name: this.local(c.name), deps: c.deps.map((d) => this.local(d)) };
  }

  /** Every cell on every sheet, named as the grid names them: for lists
   * that span the board (Needs You, what's running, find). */
  everywhere(): Cell[] {
    return [...this.all.values()].map((c) => this.viewed(c));
  }

  /** Show the sheet a grid name is on, and return its name there: `Notes!A2`
   * switches to Notes and gives `A2`. */
  reveal(name: string): string {
    const bang = name.lastIndexOf("!");
    if (bang < 0) return name;
    const full = this.full(name);
    const sheet = full.includes("!") ? full.slice(0, full.lastIndexOf("!")) : null;
    this.setSheet(sheet);
    return this.local(full);
  }

  /** Whether a cell is on the grid's sheet, or workbook-wide. */
  private inView(full: string): boolean {
    const local = this.local(full);
    return !local.includes("!");
  }

  private viewOne(full: string) {
    const c = this.all.get(full);
    if (c && this.inView(full)) this.cells.set(this.local(full), this.viewed(c));
  }

  private rebuildView() {
    this.cells = new Map();
    for (const name of this.all.keys()) this.viewOne(name);
  }



  /** By full name, so switching sheets doesn't mix up two sheets' A2. */
  private trackWork(c: Cell) {
    const working = c.state === "computing" || c.state === "queued";
    if (working && !this.workingSince.has(c.name)) this.workingSince.set(c.name, Date.now());
    if (!working) this.workingSince.delete(c.name);
  }

  /** Milliseconds a cell (by its grid name) has been working, or null when it isn't. */
  workingFor(name: string): number | null {
    const since = this.workingSince.get(this.full(name));
    return since === undefined ? null : Date.now() - since;
  }

  /** A cell by its grid name (`A2`), or on another sheet by its full one. */
  get(name: string): Cell | undefined {
    const here = this.cells.get(name);
    if (here) return here;
    const c = this.all.get(this.full(name));
    return c && this.viewed(c);
  }

  get active(): Cell | null {
    return this.selection.active ? (this.cells.get(this.selection.active) ?? null) : null;
  }

  /** The selected rectangle, when the selection is on the grid. */
  selectionRect(): [number, number, number, number] | null {
    const { active, end } = this.selection;
    if (!active || !a1Coord(active)) return null;
    return rect(active, end ?? active);
  }

  /** Every name in the selection: the rectangle, or the one named cell. */
  selectedNames(): string[] {
    const r = this.selectionRect();
    if (r) return rectNames(r);
    return this.selection.active ? [this.selection.active] : [];
  }

  select(active: string | null, end: string | null = null) {
    this.selection = { active, end: end === active ? null : end };
    this.markSeen(this.selectedNames());
    this.emit(new Set());
  }

  /** Record these cells as seen at their current write time. */
  markSeen(names: Iterable<string>) {
    for (const n of names) {
      const c = this.get(n);
      if (c) this.seen.set(this.full(n), c.provenance.at);
    }
  }

  /** The largest A1 column and row any cell occupies. */
  extent(): [number, number] {
    let cols = 0;
    let rows = 0;
    for (const name of this.cells.keys()) {
      const co = a1Coord(name);
      if (co) {
        cols = Math.max(cols, co[0]);
        rows = Math.max(rows, co[1]);
      }
    }
    return [cols, rows];
  }

  /** Cells whose deps name `name`. */
  dependentsOf(name: string): string[] {
    const out: string[] = [];
    for (const c of this.cells.values()) if (c.deps.includes(name)) out.push(c.name);
    return out;
  }

  private emit(changed: Set<string> | null) {
    for (const fn of this.listeners) fn(changed);
  }
}

export const board = new Board();
