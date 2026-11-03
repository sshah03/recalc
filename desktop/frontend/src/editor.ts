// Cell editing: one text in two synced views, the in-place input and the formula bar.
// Enter/Tab commit, Escape restores. While editing a definition, grid clicks insert
// references and F4 cycles `$` anchors. Between edits the in-place input waits,
// unseen and focused, on the selected cell, so an input method (Japanese, Chinese,
// Korean, a dead key's accent) composes there and the first composed text starts the edit.

import { a1Coord, cycleAnchors } from "./a1.js";
import { attachAutocomplete, type Autocomplete, type Typed } from "./autocomplete.js";
import { cellBox } from "./grid.js";
import { board } from "./state.js";
import { isDefinition } from "./types.js";

export type Move = "down" | "right" | "stay" | "none";

export interface EditorHandlers {
  commit(name: string, text: string, move: Move): void;
  cancelled(): void;
}

const $ = (id: string) => document.getElementById(id)!;

// Text areas, not inputs: an input drops a value's line breaks, so editing a
// multi-line answer or prompt would flatten it.
let cellInput: HTMLTextAreaElement;
let formula: HTMLTextAreaElement;
let acCell: Autocomplete;
let acFormula: Autocomplete;
let handlers: EditorHandlers;
/** The cell being edited, or null when the bar merely shows the selection. */
let editing: string | null = null;
let committing = false;
/** Where the input lives on the grid; a named cell borrows it into its row. */
let home: HTMLElement;
/** The input is waiting, unseen, on the selected cell for typing to start an edit. */
let waiting = false;
/** The calendar for a date cell, and the cell it is choosing for. */
let datePick: HTMLInputElement;
let datePickFor: string | null = null;

export function initEditor(h: EditorHandlers) {
  handlers = h;
  cellInput = $("cell-editor") as HTMLTextAreaElement;
  home = cellInput.parentElement!;
  formula = $("formula") as HTMLTextAreaElement;
  const cells = () => [...board.cells.values()].map((c) => ({ name: c.name, value: c.value }));
  // A cell with an answer type offers its values; picking one is the entry.
  const typed = (input: HTMLTextAreaElement, other: HTMLTextAreaElement): Typed => ({
    type: () => (editing ? (board.get(editing)?.answer_type ?? null) : null),
    chooseDate,
    picked: () => {
      other.value = input.value;
      commit("stay");
    },
  });
  acCell = attachAutocomplete(cellInput, $("autocomplete"), cells, () => {
    formula.value = cellInput.value;
    sizeCellInput();
  }, typed(cellInput, formula));
  acFormula = attachAutocomplete(formula, $("autocomplete"), cells, () => {
    cellInput.value = formula.value;
  }, typed(formula, cellInput));
  datePick = $("date-pick") as HTMLInputElement;
  datePick.addEventListener("change", () => {
    const name = datePickFor;
    datePickFor = null;
    if (!name || !datePick.value) return;
    if (editing === name) {
      committing = true;
      end();
      committing = false;
    }
    handlers.commit(name, datePick.value, "stay");
  });

  for (const [input, other, ac] of [
    [cellInput, formula, acCell],
    [formula, cellInput, acFormula],
  ] as const) {
    input.addEventListener("input", () => {
      if (input === cellInput && waiting && !editing) adopt();
      other.value = input.value;
      // Typed: the next click adds a reference instead of replacing the last.
      lastPick = null;
      markCode();
      if (input === cellInput) sizeCellInput();
      else sizeFormula();
    });
    input.addEventListener("focus", () => {
      // Focusing the formula bar on a selected cell starts an edit there.
      if (input === formula && !editing && board.selection.active) begin(board.selection.active, undefined, input);
    });
    input.addEventListener("blur", () => {
      // A blur that isn't a commit or a switch to the other view cancels.
      setTimeout(() => {
        if (input === cellInput && waiting && document.activeElement !== cellInput) unwait();
        if (editing && !committing && document.activeElement !== cellInput && document.activeElement !== formula) {
          cancel();
        }
      }, 0);
    });
    input.addEventListener("keydown", (e) => {
      // Keys an input method is using (Enter to accept a conversion, arrows to pick
      // one) are its own. WebKit ends the composition before the accepting Enter's
      // keydown, which then carries only key code 229.
      if (composingKey(e)) {
        e.stopPropagation();
        return;
      }
      // Waiting, not editing: the grid's own keys (arrows, Enter, Delete) handle it.
      if (!editing) return;
      // ⌥Enter (⌃Enter too) breaks the line inside the cell, as in Sheets and Excel.
      if (e.key === "Enter" && (e.altKey || e.ctrlKey) && !e.metaKey) {
        e.preventDefault();
        e.stopPropagation();
        input.setRangeText("\n", input.selectionStart, input.selectionEnd, "end");
        other.value = input.value;
        sizeCellInput();
        return;
      }
      const picking = ac.picking();
      if (ac.handleKey(e)) {
        e.preventDefault();
        other.value = input.value;
        // A typed cell's value picked with Enter or Tab is the whole entry.
        if (picking && (e.key === "Enter" || e.key === "Tab")) {
          e.stopPropagation();
          commit(e.key === "Tab" ? "right" : "down");
        }
        return;
      }
      // Consume edit-ending keys; the document handler would otherwise start editing the
      // next cell on the same Enter.
      if (e.key === "Enter") {
        e.preventDefault();
        e.stopPropagation();
        commit(e.shiftKey ? "stay" : "down");
      } else if (e.key === "Tab") {
        e.preventDefault();
        e.stopPropagation();
        commit("right");
      } else if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        cancel();
      } else if (e.key === "F4") {
        e.preventDefault();
        toggleAnchor(input);
        other.value = input.value;
      }
    });
  }
  // Focus on the grid goes to the waiting input on the selected cell, once the click
  // or call that focused it is done. A click on a cell re-renders it, so WebKit leaves
  // focus on the body; the mouseup brings it back. Keys before then reach the grid,
  // which seeds the edit itself.
  const wrap = $("grid-wrap");
  const toWaiting = () =>
    setTimeout(() => {
      const at = document.activeElement;
      if (!editing && (at === wrap || at === document.body || at === null)) wait();
    }, 0);
  wrap.addEventListener("focus", toWaiting);
  wrap.addEventListener("mouseup", toWaiting);
  cellInput.addEventListener("compositionstart", () => {
    if (waiting && !editing) adopt();
  });
}

/** The calendar for the date cell being edited; a date chosen there is the entry. */
function chooseDate() {
  if (!editing) return;
  const text = currentText().trim();
  datePick.value = /^\d{4}-\d{2}-\d{2}$/.test(text) ? text : "";
  const b = cellBox(editing);
  datePick.style.left = `${b?.x ?? 0}px`;
  datePick.style.top = `${(b?.y ?? 0) + (b?.h ?? 0)}px`;
  datePickFor = editing;
  try {
    datePick.showPicker();
  } catch {
    datePick.focus();
  }
}

/** A keydown that belongs to an input method's composition. */
export function composingKey(e: KeyboardEvent): boolean {
  return e.isComposing || e.keyCode === 229;
}

/** Put the input, unseen, on the selected cell and focus it, so typing (composed
 * or not) lands in it. A no-op while editing or with no cell selected. */
export function wait() {
  if (editing) return;
  const active = board.selection.active;
  // A named cell has no box to compose over; typing there seeds the formula bar.
  if (!active || !a1Coord(active)) {
    unwait();
    return;
  }
  waiting = true;
  cellInput.value = "";
  cellInput.classList.add("waiting");
  cellInput.setAttribute("aria-label", `Type to edit ${active}`);
  cellInput.hidden = false;
  placeWaiting();
  if (document.activeElement !== cellInput) cellInput.focus({ preventScroll: true });
}

function unwait() {
  if (!waiting) return;
  waiting = false;
  cellInput.classList.remove("waiting");
  cellInput.removeAttribute("aria-label");
  if (!editing) cellInput.hidden = true;
}

/** The input is focused and waiting: keys typed now go into it, not to the grid. */
export function isWaiting(): boolean {
  return waiting && !editing && document.activeElement === cellInput;
}

/** Typing reached the waiting input: that text starts an edit of the selected cell,
 * replacing its content. The input keeps its value (a composition stays live). */
function adopt() {
  const active = board.selection.active;
  if (!active) return;
  waiting = false;
  cellInput.classList.remove("waiting");
  cellInput.removeAttribute("aria-label");
  editing = active;
  formula.value = cellInput.value;
  markEditing();
  positionEditor();
  acCell.refresh();
}

function placeWaiting() {
  const active = board.selection.active;
  const b = active ? cellBox(active) : null;
  // Off screen or a named cell: the input still takes the keys, from the corner.
  cellInput.style.left = `${b?.x ?? 0}px`;
  cellInput.style.top = `${b?.y ?? 0}px`;
  cellInput.style.height = `${b?.h ?? 22}px`;
  cellInput.style.minWidth = `${b?.w ?? 80}px`;
  cellInput.style.width = `${b?.w ?? 80}px`;
}

export function isEditing(): boolean {
  return editing !== null;
}

/** Editing a definition (=, $, !, @) with the caret where a reference fits:
 * inside a quoted prompt, or after an operator, bracket, comma or space. A
 * click elsewhere (after the closing parenthesis, say) ends the edit. */
export function pickingRefs(): boolean {
  if (editing === null) return false;
  const input = activeInput();
  const text = input.value;
  // Only a definition takes references; an empty or plain value (or a $
  // amount) never does.
  if (!isDefinition(text)) return false;
  if (pickStart !== null) return true;
  // Right after a picked reference, a click replaces it, as in Sheets.
  if (lastPick && input.selectionStart === lastPick.end && input.selectionEnd === lastPick.end) return true;
  const before = text.slice(0, input.selectionStart ?? text.length);
  if ((before.match(/'/g) ?? []).length % 2 === 1 || (before.match(/"/g) ?? []).length % 2 === 1) return true;
  return !/[\w)\]}.]$/.test(before);
}

/** Commit an open edit where it stands; a grid click does this before selecting. */
export function commitOpen() {
  commit("none");
}

function currentText(): string {
  return document.activeElement === formula ? formula.value : cellInput.value;
}

function activeInput(): HTMLTextAreaElement {
  return document.activeElement === formula ? formula : cellInput;
}

/** Start editing `name`. A seed replaces the content (typing into a cell);
 * otherwise the cell's own content is offered. */
export function beginEdit(name: string, seed?: string) {
  begin(name, seed, cellInput);
}

function begin(name: string, seed: string | undefined, where: HTMLTextAreaElement) {
  waiting = false;
  lastPick = null;
  cellInput.classList.remove("waiting");
  cellInput.removeAttribute("aria-label");
  editing = name;
  const cell = board.get(name);
  const text = seed ?? (cell ? (cell.source ?? cell.value) : "");
  cellInput.value = text;
  formula.value = text;
  cellInput.hidden = false;
  markEditing();
  // Places the input, or hides it where it has nowhere to sit.
  positionEditor();
  const input = cellInput.hidden ? formula : where;
  if (document.activeElement !== input) input.focus({ preventScroll: true });
  input.setSelectionRange(text.length, text.length);
  (input === formula ? acFormula : acCell).refresh();
}

/** Say an edit is open: the window's look (body.editing), the status bar's
 * keys, and the code font only for a definition. */
function markEditing() {
  document.body.classList.toggle("editing", editing !== null);
  const hint = document.getElementById("status-editing");
  if (hint) {
    hint.hidden = editing === null;
    hint.textContent = editing ? `Editing ${editing} · Enter saves · Esc cancels · ⌥Enter adds a line` : "";
  }
  markCode();
}

/** Formulas, commands and prompts read better in the code font; a value
 * keeps the cell's own. */
function markCode() {
  cellInput.classList.toggle("code", isDefinition(cellInput.value));
}

/** The side panel's row for a named cell, when it's showing. */
function namedRow(name: string): HTMLElement | null {
  return [...document.querySelectorAll<HTMLElement>("#named .named-row")].find((r) => r.dataset.name === name) ?? null;
}

/** The formula bar took the edit because the cell had no row yet. */
let inBarForNow = false;

/** Edit a named cell in its own row, in place of its value. */
function dock(row: HTMLElement) {
  if (cellInput.parentElement !== row) {
    undock();
    row.appendChild(cellInput);
  }
  // Its row has appeared: the typing moves there, the caret where it was.
  if (inBarForNow && document.activeElement === formula) {
    const at = formula.selectionStart ?? formula.value.length;
    cellInput.value = formula.value;
    cellInput.hidden = false;
    cellInput.focus({ preventScroll: true });
    cellInput.setSelectionRange(at, at);
  }
  inBarForNow = false;
  row.classList.add("editing");
  cellInput.classList.add("in-named");
  for (const p of ["left", "top", "width", "minWidth"] as const) cellInput.style[p] = "";
  cellInput.hidden = false;
  sizeCellInput();
}

function undock() {
  if (!home || cellInput.parentElement === home) return;
  cellInput.parentElement?.classList.remove("editing");
  cellInput.classList.remove("in-named");
  home.appendChild(cellInput);
}

/** Keep the floating input on its cell through scrolls and re-renders. */
export function positionEditor() {
  if (waiting && !editing) placeWaiting();
  if (!editing) return;
  const b = cellBox(editing);
  if (!b) {
    const row = namedRow(editing);
    if (row) {
      dock(row);
      return;
    }
    // A named cell with its list folded, or not listed yet: the formula bar is the editor.
    undock();
    inBarForNow = true;
    cellInput.hidden = true;
    if (document.activeElement !== formula) formula.focus({ preventScroll: true });
    return;
  }
  undock();
  cellInput.style.left = `${b.x}px`;
  cellInput.style.top = `${b.y}px`;
  cellInput.style.height = `${b.h}px`;
  cellInput.style.minWidth = `${b.w}px`;
  sizeCellInput();
}

function sizeCellInput() {
  // Grow with the text so a long formula stays readable, and down a line per line break.
  const b = editing ? cellBox(editing) : null;
  const lines = cellInput.value.split("\n");
  if (cellInput.classList.contains("in-named")) {
    // The row's width; down a line per line break.
    cellInput.style.height = `${Math.max(24, Math.min(lines.length, 6) * 17 + 8)}px`;
    sizeFormula();
    return;
  }
  const min = b?.w ?? 120;
  const chars = Math.max(...lines.map((l) => l.length));
  cellInput.style.width = `${Math.max(min, Math.min(chars * 7.5 + 16, 720))}px`;
  cellInput.style.height = `${Math.max(b?.h ?? 28, Math.min(lines.length, 12) * 17 + 10)}px`;
  sizeFormula();
}

/** The formula bar shows one line; editing a multi-line text it grows to six. */
function sizeFormula() {
  const lines = editing ? formula.value.split("\n").length : 1;
  formula.style.height = lines > 1 ? `${Math.min(lines, 6) * 16 + 16}px` : "";
}

function commit(move: Move) {
  if (!editing) return;
  const name = editing;
  const text = currentText();
  committing = true;
  end();
  committing = false;
  handlers.commit(name, text, move);
}

export function cancel() {
  if (!editing) return;
  end();
  handlers.cancelled();
}

function end() {
  editing = null;
  inBarForNow = false;
  lastPick = null;
  markEditing();
  cellInput.hidden = true;
  undock();
  cellInput.blur();
  formula.blur();
  acCell.close();
  acFormula.close();
  syncFormulaBar();
}

/** When not editing, the bar shows the selected cell's content, read to edit. */
export function syncFormulaBar() {
  if (editing) return;
  const { active, end: selEnd } = board.selection;
  const cell = board.active;
  $("namebox").textContent = active && selEnd ? `${active}:${selEnd}` : (active ?? "");
  formula.value = cell ? (cell.source ?? cell.value) : "";
  formula.disabled = !active;
  formula.placeholder = active ? "A value, or =formula, $command, !action, @agent('…')" : "Select a cell";
  sizeFormula();
}

/** A reference picked on the grid while editing a definition. `done`
 * finalizes the drag; before that the text previews the range. */
let pickStart: number | null = null;
/** The reference the last click put in, until the next keystroke: another
 * click replaces it rather than ending the edit. */
let lastPick: { start: number; end: number } | null = null;
let pickEnd: number | null = null;
export function insertRef(text: string, done: boolean) {
  const input = activeInput();
  if (pickStart === null) {
    const at = input.selectionStart ?? input.value.length;
    if (lastPick && at === lastPick.end && input.selectionEnd === at) {
      pickStart = lastPick.start;
      pickEnd = lastPick.end;
    } else {
      pickStart = at;
      pickEnd = input.selectionEnd ?? pickStart;
      // Right after a typed '{', don't double the brace.
      if (input.value.slice(0, pickStart).endsWith("{") && text.startsWith("{")) pickStart -= 1;
    }
  }
  const before = input.value.slice(0, pickStart);
  const after = input.value.slice(pickEnd!);
  input.value = before + text + after;
  const caret = pickStart + text.length;
  input.setSelectionRange(caret, caret);
  (input === formula ? cellInput : formula).value = input.value;
  if (done) {
    lastPick = { start: pickStart, end: caret };
    pickStart = null;
    pickEnd = null;
    input.focus();
    if (input === cellInput) sizeCellInput();
  } else {
    // Keep replacing the same span as the drag grows.
    pickEnd = caret;
  }
}

/** F4: cycle `$` anchors on the A1 reference around the caret. */
function toggleAnchor(input: HTMLTextAreaElement) {
  const text = input.value;
  const caret = input.selectionStart ?? text.length;
  // Find the enclosing {...} body, or the bare A1 token at the caret.
  const open = text.lastIndexOf("{", caret);
  const close = text.indexOf("}", Math.max(0, caret - 1));
  let from: number;
  let to: number;
  if (open !== -1 && close !== -1 && open < close && !text.slice(open, close).includes(" ")) {
    from = open + 1;
    to = close;
  } else {
    const re = /\$?[A-Z]+\$?[0-9]+/g;
    let m: RegExpExecArray | null;
    from = -1;
    to = -1;
    while ((m = re.exec(text))) {
      if (m.index <= caret && caret <= m.index + m[0].length) {
        from = m.index;
        to = m.index + m[0].length;
        break;
      }
    }
    if (from === -1) return;
  }
  const body = text.slice(from, to);
  // Cycle every A1 ref in the body: `A1:B3` toggles both corners together.
  const cycled = body.replace(/\$?[A-Z]+\$?[0-9]+/g, (ref) => cycleAnchors(ref));
  if (cycled === body) return;
  input.value = text.slice(0, from) + cycled + text.slice(to);
  const caretNow = Math.min(caret, from + cycled.length);
  input.setSelectionRange(caretNow, caretNow);
}
