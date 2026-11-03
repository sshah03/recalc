// The sheet tabs under the grid: one per sheet, the shown one marked; click
// to switch, double-click to rename, right-click for the rest, + to add.

import { showContextMenu, confirm } from "./chrome.js";
import { board } from "./state.js";

export interface TabHandlers {
  /** Show sheet `sheet` (null: the first). */
  show(sheet: string | null): void;
  add(): void;
  rename(from: string, to: string): void;
  remove(sheet: string): void;
  move(sheet: string, index: number): void;
  duplicate(sheet: string): void;
}

let handlers: TabHandlers;

export function initTabs(h: TabHandlers) {
  handlers = h;
}

let drawn = "";

/** Draw the tabs when the sheets or the shown one changed (or `force`). */
export function renderTabs(force = false) {
  const strip = document.getElementById("sheet-tabs");
  if (!strip) return;
  const key = JSON.stringify([board.sheets, board.sheet]);
  if (key === drawn && !force) return;
  drawn = key;
  strip.replaceChildren();
  const names = [board.sheets.first, ...board.sheets.others];
  names.forEach((name, i) => {
    const tab = document.createElement("button");
    const first = i === 0;
    const shown = first ? board.sheet === null : board.sheet === name;
    tab.className = "sheet-tab" + (shown ? " active" : "");
    tab.setAttribute("role", "tab");
    tab.setAttribute("aria-selected", String(shown));
    tab.textContent = name;
    tab.title = shown ? "Double-click to rename" : `Show ${name}`;
    tab.onclick = () => handlers.show(first ? null : name);
    tab.ondblclick = () => startRename(tab, name);
    tab.oncontextmenu = (e) => {
      e.preventDefault();
      handlers.show(first ? null : name);
      showContextMenu(e.clientX, e.clientY, [
        { label: "Rename…", run: () => startRename(tab, name) },
        { label: "Move Left", run: () => handlers.move(name, i - 2), disabled: first || i === 1 },
        { label: "Move Right", run: () => handlers.move(name, i), disabled: first || i === names.length - 1 },
        { label: "Duplicate Sheet…", run: () => handlers.duplicate(name) },
        null,
        {
          label: "Delete Sheet…",
          disabled: first,
          run: () =>
            void confirm(`Delete ${name}?`, `Its cells go, and references to them become #REF!. This can't be undone.`, "Delete").then(
              (yes) => yes && handlers.remove(name),
            ),
        },
      ]);
    };
    strip.appendChild(tab);
  });
  const add = document.createElement("button");
  add.className = "sheet-tab add";
  add.textContent = "+";
  add.title = "Add a sheet";
  add.setAttribute("aria-label", "Add a sheet");
  add.onclick = () => handlers.add();
  strip.appendChild(add);
}

/** Rename in place: the tab becomes a field until Enter or a click away. */
function startRename(tab: HTMLElement, name: string) {
  const input = document.createElement("input");
  input.className = "sheet-tab rename";
  input.value = name;
  input.setAttribute("aria-label", `Rename ${name}`);
  let done = false;
  const finish = (save: boolean) => {
    if (done) return;
    done = true;
    const to = input.value.trim();
    if (save && to && to !== name) handlers.rename(name, to);
    else renderTabs(true);
  };
  input.onkeydown = (e) => {
    if (e.key === "Enter") finish(true);
    if (e.key === "Escape") finish(false);
    e.stopPropagation();
  };
  input.onblur = () => finish(true);
  tab.replaceWith(input);
  input.focus();
  input.select();
}

/** The next free name like Excel's: Sheet2, Sheet3, … */
export function nextSheetName(): string {
  const taken = new Set([board.sheets.first, ...board.sheets.others].map((s) => s.toLowerCase()));
  for (let i = 2; ; i++) if (!taken.has(`sheet${i}`)) return `Sheet${i}`;
}
