// Editor autocomplete: '=' offers functions with signatures, '{' cell references,
// '@' agent calls. In a cell with an answer type, a value offers what the cell takes:
// its options, Yes or No, or the date you typed as YYYY-MM-DD. One dropdown element,
// placed under whichever editor has focus.

import { FUNCTIONS } from "./help.js";
import { board, quoteSheet } from "./state.js";
import type { AnswerType } from "./types.js";
import { isDefinition } from "./types.js";

interface Item {
  /** Text that replaces the [from, to) span on accept. */
  insert: string;
  label: string;
  hint: string;
  /** Run instead of inserting (the calendar). */
  action?: () => void;
}

interface Suggestions {
  items: Item[];
  from: number;
  to: number;
  /** A value for a typed cell: accepting it is the whole entry. */
  pick?: boolean;
  /** The item highlighted at first; -1 for none, so Enter keeps what's typed. */
  start?: number;
}

export interface Autocomplete {
  refresh(): void;
  close(): void;
  /** True when the dropdown consumed the key. */
  handleKey(e: KeyboardEvent): boolean;
  /** Enter or Tab now would pick a typed cell's value (not the calendar). */
  picking(): boolean;
}

/** What a typed cell's value offers: the cell's answer type, and the calendar. */
export interface Typed {
  type: () => AnswerType | null;
  chooseDate: () => void;
  /** A value was picked with the mouse: that's the entry. */
  picked: () => void;
}

// Builtin agents; custom ones come from [agents] in .recalc/config.toml.
// Explicit-run, like ! actions.
const AGENTS = ["claude", "codex", "pi"].map((name) => ({
  insert: `${name}('`,
  label: `@${name}('prompt', {A1:B3})`,
  hint: `runs ${name} connected to this board, only when you run it`,
}));

export function attachAutocomplete(
  input: HTMLInputElement | HTMLTextAreaElement,
  box: HTMLElement,
  cells: () => Array<{ name: string; value: string }>,
  /** Called after an accepted completion changes the input's text. */
  onAccept: () => void = () => {},
  typed?: Typed,
): Autocomplete {
  let current: Suggestions | null = null;
  let active = 0;

  function suggest(): Suggestions | null {
    const text = input.value;
    const caret = input.selectionStart ?? text.length;
    const before = text.slice(0, caret);
    const first = text.trimStart()[0];

    const type = typed?.type();
    const definition = isDefinition(text);
    if (type && !definition) return typedValues(type, text);

    // Inside an unclosed {…}: cell references, in any definition kind.
    const open = before.lastIndexOf("{");
    if (definition && open !== -1 && !before.slice(open).includes("}")) {
      const prefix = before.slice(open + 1);
      const bang = prefix.lastIndexOf("!");
      if (bang !== -1) {
        // `{Rules!A`: the cells of that sheet.
        const sheet = prefix.slice(0, bang).replace(/^'(.*)'$/, "$1").replace(/''/g, "'").toLowerCase();
        const rest = prefix.slice(bang + 1).replace(/\$/g, "").toLowerCase();
        const items = board
          .everywhere()
          .map((c) => ({ c, at: c.name.lastIndexOf("!") }))
          .filter(({ c, at }) => at !== -1 && c.name.slice(0, at).toLowerCase() === sheet && c.name.slice(at + 1).toLowerCase().startsWith(rest))
          .slice(0, 40)
          .map(({ c, at }) => ({
            insert: `${quoteSheet(c.name.slice(0, at))}!${c.name.slice(at + 1)}}`,
            label: `{${quoteSheet(c.name.slice(0, at))}!${c.name.slice(at + 1)}}`,
            hint: c.value.split("\n")[0],
          }));
        return items.length ? { items, from: open + 1, to: caret } : null;
      }
      // The other sheets, by name, before this one's cells.
      const p = prefix.replace(/^'/, "").replace(/\$/g, "").toLowerCase();
      const sheets = [board.sheets.first, ...board.sheets.others]
        .filter((name) => name !== board.sheetName && name.toLowerCase().startsWith(p))
        .map((name) => ({ insert: `${quoteSheet(name)}!`, label: `{${quoteSheet(name)}!…}`, hint: `a cell on ${name}` }));
      if (!/^\$?[A-Za-z0-9_$]*$/.test(prefix)) return sheets.length ? { items: sheets, from: open + 1, to: caret } : null;
      const here = cells()
        .filter((c) => c.name.toLowerCase().startsWith(p))
        .slice(0, 40)
        .map((c) => ({
          insert: c.name + "}",
          label: `{${c.name}}`,
          hint: c.value.split("\n")[0],
        }));
      const items = [...sheets, ...here];
      return items.length ? { items, from: open + 1, to: caret } : null;
    }

    // Typing the agent name after '@': complete the call form.
    if (first === "@") {
      const namePart = before.slice(before.indexOf("@") + 1);
      if (!/^[A-Za-z_-]*$/.test(namePart)) return null;
      const p = namePart.toLowerCase();
      const items = AGENTS.filter((a) => a.insert.startsWith(p));
      return items.length ? { items, from: caret - namePart.length, to: caret } : null;
    }

    if (first === "=") {
      const m = /[A-Za-z_][A-Za-z0-9_]*$/.exec(before);
      const token = m ? m[0].toLowerCase() : "";
      // A name in progress offers the functions it starts. The bare "=" offers
      // nothing, as in Sheets: the next thing is often a click on a cell, and
      // a list over the grid would take that click.
      if (!m) return null;
      const items = FUNCTIONS.filter((f) => f.name.startsWith(token)).map((f) => ({
        insert: f.name + "(",
        label: f.sig,
        hint: f.what,
      }));
      return items.length ? { items, from: caret - token.length, to: caret } : null;
    }
    return null;
  }

  /** The values a typed cell takes, for what's typed so far. */
  function typedValues(type: AnswerType, text: string): Suggestions | null {
    const q = text.trim().toLowerCase();
    const span = { from: 0, to: text.length, pick: true };
    if (type.kind === "choice" || type.kind === "yesno") {
      const options = type.kind === "yesno" ? ["Yes", "No"] : (type.options ?? []);
      const exact = options.findIndex((o) => o.trim().toLowerCase() === q);
      // The cell's own value lists them all, itself highlighted; typing narrows.
      const shown = exact !== -1 ? options : options.filter((o) => o.toLowerCase().startsWith(q));
      if (!shown.length) return null;
      const items = shown.map((o) => ({ insert: o, label: o, hint: "" }));
      return { items, ...span, start: exact !== -1 ? exact : q ? 0 : -1 };
    }
    if (type.kind === "date") {
      const calendar: Item = { insert: "", label: "Choose from a calendar…", hint: "", action: () => typed!.chooseDate() };
      if (!q) return { items: [{ insert: isoDate(new Date()), label: isoDate(new Date()), hint: "today" }, calendar], ...span, start: -1 };
      if (/^\d{4}-\d{2}-\d{2}$/.test(q)) return { items: [calendar], ...span, start: -1 };
      const iso = parseDate(q);
      return iso ? { items: [{ insert: iso, label: iso, hint: "the date you typed" }, calendar], ...span, start: 0 } : { items: [calendar], ...span, start: -1 };
    }
    return null;
  }

  function render() {
    if (document.activeElement !== input && current) current = null;
    box.hidden = current === null;
    if (!current) return;
    box.replaceChildren();
    const r = input.getBoundingClientRect();
    box.style.left = `${Math.max(0, r.left)}px`;
    box.style.top = `${r.bottom}px`;
    current.items.forEach((item, i) => {
      const row = document.createElement("div");
      row.className = "ac-row" + (i === active ? " active" : "");
      const name = document.createElement("span");
      name.className = "ac-name";
      name.textContent = item.label;
      const hint = document.createElement("span");
      hint.className = "ac-hint";
      hint.textContent = item.hint;
      row.append(name, hint);
      // mousedown, not click: the input must keep focus.
      row.addEventListener("mousedown", (e) => {
        e.preventDefault();
        const pick = current?.pick && !item.action;
        accept(item);
        if (pick) typed?.picked();
      });
      box.appendChild(row);
    });
  }

  function accept(item: Item) {
    if (!current) return;
    if (item.action) {
      close();
      item.action();
      return;
    }
    const text = input.value;
    input.value = text.slice(0, current.from) + item.insert + text.slice(current.to);
    const caret = current.from + item.insert.length;
    input.setSelectionRange(caret, caret);
    onAccept();
    refresh();
  }

  function refresh() {
    current = suggest();
    active = current?.start ?? 0;
    render();
  }

  function close() {
    current = null;
    render();
  }

  function handleKey(e: KeyboardEvent): boolean {
    if (!current) return false;
    if (e.key === "Escape") {
      close();
      return true;
    }
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      const n = current.items.length;
      active = active === -1 ? (e.key === "ArrowDown" ? 0 : n - 1) : (active + (e.key === "ArrowDown" ? 1 : n - 1)) % n;
      render();
      return true;
    }
    // Nothing highlighted: Enter and Tab keep what's typed.
    if ((e.key === "Tab" || e.key === "Enter") && active !== -1) {
      accept(current.items[active]);
      return true;
    }
    return false;
  }

  input.addEventListener("input", refresh);
  const picking = () => !!current?.pick && active !== -1 && !current.items[active]?.action;
  return { refresh, close, handleKey, picking };
}

function isoDate(d: Date): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

const MONTHS = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];

/** A date as people type it, as YYYY-MM-DD: today, yesterday, tomorrow, "oct 3",
 * "3 October 2026", "10/3/26" (day and month in the order this computer writes
 * them). Null when it isn't one. */
export function parseDate(text: string, now = new Date()): string | null {
  const t = text.trim().toLowerCase().replace(/,/g, " ").replace(/\s+/g, " ");
  const shift = { today: 0, yesterday: -1, tomorrow: 1 }[t];
  if (shift !== undefined) return isoDate(new Date(now.getFullYear(), now.getMonth(), now.getDate() + shift));
  const year = (y: string | undefined) => (y === undefined ? now.getFullYear() : y.length <= 2 ? 2000 + Number(y) : Number(y));
  const make = (y: number, m: number, d: number) => {
    const date = new Date(y, m - 1, d);
    return date.getFullYear() === y && date.getMonth() === m - 1 && date.getDate() === d ? isoDate(date) : null;
  };
  const month = (w: string) => MONTHS.findIndex((m) => w.startsWith(m) && m.length >= 3) + 1;
  let m = /^([a-z]+)\.? (\d{1,2})(?:st|nd|rd|th)?(?: (\d{2}|\d{4}))?$/.exec(t);
  if (m && month(m[1])) return make(year(m[3]), month(m[1]), Number(m[2]));
  m = /^(\d{1,2})(?:st|nd|rd|th)? ([a-z]+)\.?(?: (\d{2}|\d{4}))?$/.exec(t);
  if (m && month(m[2])) return make(year(m[3]), month(m[2]), Number(m[1]));
  m = /^(\d{1,2})[/.-](\d{1,2})(?:[/.-](\d{2}|\d{4}))?$/.exec(t);
  if (m) {
    const [a, b] = [Number(m[1]), Number(m[2])];
    return dayFirst() ? make(year(m[3]), b, a) : make(year(m[3]), a, b);
  }
  return null;
}

/** Whether this computer writes the day before the month (3/10 for 3 October). */
function dayFirst(): boolean {
  try {
    const parts = new Intl.DateTimeFormat(undefined).formatToParts(new Date(2026, 9, 3));
    return parts.findIndex((p) => p.type === "day") < parts.findIndex((p) => p.type === "month");
  } catch {
    return false;
  }
}
