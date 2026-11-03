// Every user-facing action, defined once. The native menu (src-tauri/src/menu.rs)
// emits these ids and owns the shortcuts; palette, toolbar and context menu pick from them.

import * as chrome from "./chrome.js";
import * as onboard from "./onboard.js";
import { board } from "./state.js";
import * as api from "./tauri.js";
import { isRunnable } from "./types.js";

export interface Action {
  id: string;
  label: string;
  menu: "File" | "Edit" | "Format" | "View" | "Cell" | "Agent" | "Help";
  shortcut?: string;
  /** Longer tooltip; the label is the default. */
  title?: string;
  /** Grayed out when false. Omitted = always available. */
  enabled?: () => boolean;
  run: () => void;
}

/** What actions need from the app: editor, selection, error reporting. */
export interface ActionCtx {
  editSelected(seed?: string): void;
  newCell(): void;
  runSelected(): void;
  runSelection(): void;
  driverAnswer(): void;
  /** The board's ! cells, for the palette: "Run normalize_dates". */
  boardScripts(): Array<{ label: string; note: string; run: () => void }>;
  driverRunning(): boolean;
  stopSelected(): void;
  stopAll(): void;
  setBudget(): void;
  copyMarkdown(): void;
  pasteValues(): void;
  find(): void;
  toggleWrap(): void;
  toggleNotify(): void;
  freeze(): void;
  unfreeze(): void;
  setFilter(which: "all" | "flagged" | "unverified" | "errors" | "running"): void;
  filterToValue(): void;
  filterColumn(): void;
  restructure(axis: "rows" | "cols", insert: boolean, after: boolean): void;
  sort(descending: boolean): void;
  format(patch: Partial<import("./types.js").Format> | "clear" | "header" | "fill" | "more" | "number" | "bold" | "italic"): void;
  answerTypeSelection(): void;
  /** The start screen, to make a new board. */
  openNew(): void;
  /** A new board (the open one when it's empty), then `fill` on it. */
  startNew(fill?: () => unknown): void;
  addQuestionColumn(): void;
  runColumn(): void;
  toggleSource(): void;
  openTerminalSelected(): void;
  openSessionSelected(): void;
  endSessionSelected(): void;
  handoffSelected(): void;
  showDriver(): void;
  toggleWideDriver(): void;
  notifySelected(): void;
  toggleVerified(): void;
  flagSelected(): void;
  nextUnverified(): void;
  verifyAndNext(): void;
  startReview(): void;
  toggleSpark(): void;
  deleteSelected(): void;
  fillSelection(dir: "down" | "right"): void;
  undo(): void;
  redo(): void;
  selectAll(): void;
  showDecisions(): void;
  oops(e: unknown): void;
}

let actions: Action[] = [];
let enabledSent = new Map<string, boolean>();

export function allActions(): Action[] {
  return actions;
}

export function actionById(id: string): Action | undefined {
  return actions.find((a) => a.id === id);
}

export function runAction(id: string) {
  const a = actionById(id);
  if (!a) return;
  if (a.enabled && !a.enabled()) return;
  a.run();
}

export function initActions(ctx: ActionCtx) {
  const hasSel = () => board.selection.active !== null;
  const hasCell = () => board.active !== null;
  const onGrid = () => board.selectionRect() !== null;
  const isAgentCell = () => board.active?.kind === "agent_call";
  const runnable = () => {
    const c = board.active;
    return c !== null && isRunnable(c.kind);
  };
  const anyAgent = () => board.selectedNames().some((n) => board.get(n)?.kind === "agent_call");
  const anyRunnable = () => board.selectedNames().some((n) => {
    const c = board.get(n);
    return c !== undefined && isRunnable(c.kind);
  });
  // A value to sign off or a sign-off to undo; verifying an unrun cell would lock an empty answer.
  const verifiable = () => board.selectedNames().some((n) => {
    const c = board.get(n);
    return !!c && (c.value !== "" || c.mark?.kind === "verified");
  });
  const anchor = () => board.selectedNames()[0] ?? "A1";

  actions = [
    { id: "file.new_board", menu: "File", label: "New Board…", shortcut: "⌘N", title: "Start a blank board, a document grid, an example or a template", run: ctx.openNew },
    { id: "file.open", menu: "File", label: "Open…", shortcut: "⌘O", title: "Open a board, or a board someone exported to one file", run: () => void api.openFile().catch(ctx.oops) },
    { id: "file.open_board", menu: "File", label: "Open Folder…", title: "Open the board in a folder, such as a repo", run: () => void api.openBoard().catch(ctx.oops) },
    { id: "file.open_recent", menu: "File", label: "Open Recent…", run: () => void chrome.openRecents(ctx.oops) },
    { id: "file.template", menu: "File", label: "Insert Template…", title: "Add an example board's cells to this one", run: () => void chrome.openGallery() },
    { id: "file.import_csv", menu: "File", label: "Import CSV…", title: "Paste a CSV file's cells in, starting at the selection", run: () => void api.importCsv(anchor()).catch(ctx.oops) },
    {
      id: "file.import_files",
      menu: "File",
      label: "Import Files as Rows…",
      title: "List a folder's files down the column from the selection, one document per row",
      run: () => void api.importFiles(anchor()).catch(ctx.oops),
    },
    { id: "file.save", menu: "File", label: "Save", shortcut: "⌘S", title: "Boards save every change as you make it; this confirms it", run: () => chrome.toast("Saved. recalc saves every change as you make it.") },
    {
      id: "file.save_copy",
      menu: "File",
      label: "Save a Copy…",
      shortcut: "⇧⌘S",
      title: "Copy this board to a new name and place, then work in the copy",
      run: () =>
        void api
          .saveCopy()
          .then((said) => said && chrome.toast(said))
          .catch(ctx.oops),
    },
    { id: "file.rename", menu: "File", label: "Rename…", title: "Give this board's folder a new name", run: () => chrome.openBoardInfo() },
    {
      id: "file.move",
      menu: "File",
      label: "Move To…",
      title: "Move this board's folder somewhere else",
      run: () => void api.moveBoard().catch(ctx.oops),
    },
    {
      id: "file.export_board",
      menu: "File",
      label: "Export Board…",
      title: "Write a copy of the board, and the documents it names, to one file you can send",
      run: () =>
        void api
          .exportBoard()
          .then((said) => said && chrome.notice("Board exported", said))
          .catch(ctx.oops),
    },
    { id: "file.export_csv", menu: "File", label: "Export CSV…", run: () => void api.exportCsv().catch(ctx.oops) },
    {
      id: "file.export_xlsx",
      menu: "File",
      label: "Export Excel Workbook…",
      title: "Save values with their formats; citations and marks become cell notes",
      run: () => void api.exportXlsx().catch(ctx.oops),
    },
    { id: "file.save_template", menu: "File", label: "Save Board as Template…", run: () => void api.saveBoardAs().catch(ctx.oops) },

    { id: "edit.undo", menu: "Edit", label: "Undo", shortcut: "⌘Z", run: ctx.undo },
    { id: "edit.redo", menu: "Edit", label: "Redo", shortcut: "⇧⌘Z", run: ctx.redo },
    { id: "edit.select_all", menu: "Edit", label: "Select All", shortcut: "⌘A", title: "Select every filled cell; in a text field, its text", run: ctx.selectAll },
    { id: "edit.paste_values", menu: "Edit", label: "Paste Values", shortcut: "⇧⌘V", title: "Paste what you copied as plain values, not the formulas or agents behind them", enabled: onGrid, run: ctx.pasteValues },
    { id: "edit.copy_markdown", menu: "Edit", label: "Copy as Markdown Table", shortcut: "⇧⌘C", title: "Copy the selection as a Markdown table, first row as the header", enabled: onGrid, run: ctx.copyMarkdown },
    { id: "edit.delete", menu: "Edit", label: "Delete Cells", shortcut: "⌫", enabled: hasCell, run: ctx.deleteSelected },
    { id: "edit.insert_row", menu: "Edit", label: "Insert Row Above", enabled: onGrid, run: () => ctx.restructure("rows", true, false) },
    { id: "edit.insert_row_below", menu: "Edit", label: "Insert Row Below", enabled: onGrid, run: () => ctx.restructure("rows", true, true) },
    { id: "edit.insert_col", menu: "Edit", label: "Insert Column Left", enabled: onGrid, run: () => ctx.restructure("cols", true, false) },
    { id: "edit.insert_col_right", menu: "Edit", label: "Insert Column Right", enabled: onGrid, run: () => ctx.restructure("cols", true, true) },
    { id: "edit.delete_rows", menu: "Edit", label: "Delete Rows", enabled: onGrid, run: () => ctx.restructure("rows", false, false) },
    { id: "edit.delete_cols", menu: "Edit", label: "Delete Columns", enabled: onGrid, run: () => ctx.restructure("cols", false, false) },
    { id: "edit.sort_asc", menu: "Edit", label: "Sort Rows A → Z", title: "Sort the selected rows, or every row below 1, by the active column: numbers, then text, blanks last", enabled: onGrid, run: () => ctx.sort(false) },
    { id: "edit.sort_desc", menu: "Edit", label: "Sort Rows Z → A", title: "Sort the selected rows, or every row below 1, by the active column, highest first; blanks stay last", enabled: onGrid, run: () => ctx.sort(true) },
    { id: "edit.find", menu: "Edit", label: "Find…", shortcut: "⌘F", run: ctx.find },

    { id: "format.bold", menu: "Format", label: "Bold", shortcut: "⌘B", enabled: hasSel, run: () => ctx.format("bold") },
    { id: "format.italic", menu: "Format", label: "Italic", shortcut: "⌘I", enabled: hasSel, run: () => ctx.format("italic") },
    { id: "format.align_left", menu: "Format", label: "Align Left", shortcut: "⇧⌘L", enabled: hasSel, run: () => ctx.format({ align: "left" }) },
    { id: "format.align_center", menu: "Format", label: "Align Center", shortcut: "⇧⌘E", enabled: hasSel, run: () => ctx.format({ align: "center" }) },
    { id: "format.align_right", menu: "Format", label: "Align Right", shortcut: "⇧⌘R", enabled: hasSel, run: () => ctx.format({ align: "right" }) },
    { id: "format.num_auto", menu: "Format", label: "Automatic", title: "Show the value as typed", enabled: hasSel, run: () => ctx.format({ num: undefined }) },
    { id: "format.num_text", menu: "Format", label: "Plain Text", title: "Never treat the value as a number; align it left", enabled: hasSel, run: () => ctx.format({ num: "text" }) },
    { id: "format.num_number", menu: "Format", label: "Number", title: "1,000.12", enabled: hasSel, run: () => ctx.format({ num: "number" }) },
    { id: "format.num_integer", menu: "Format", label: "Integer", title: "1,000", enabled: hasSel, run: () => ctx.format({ num: "integer" }) },
    { id: "format.num_currency", menu: "Format", label: "Currency", title: "$1,000.12", enabled: hasSel, run: () => ctx.format({ num: "currency" }) },
    { id: "format.num_percent", menu: "Format", label: "Percent", title: "10.12%", enabled: hasSel, run: () => ctx.format({ num: "percent" }) },
    { id: "format.num_date", menu: "Format", label: "Date", title: "2026-09-03", enabled: hasSel, run: () => ctx.format({ num: "date" }) },
    { id: "format.fill", menu: "Format", label: "Fill Color…", enabled: hasSel, run: () => ctx.format("fill") },
    { id: "format.header", menu: "Format", label: "Make Header Row", title: "Bold row 1 and keep it in view; sorting leaves it in place", run: () => ctx.format("header") },
    { id: "format.more", menu: "Format", label: "Format Cells…", shortcut: "⌘1", enabled: hasSel, run: () => ctx.format("more") },
    { id: "format.number", menu: "Format", label: "Number Format…", title: "Pick how numbers are shown; each choice previews this cell's value", enabled: hasSel, run: () => ctx.format("number") },
    { id: "format.clear", menu: "Format", label: "Clear Formatting", shortcut: "⌘\\", enabled: hasSel, run: () => ctx.format("clear") },
    { id: "edit.fill_down", menu: "Edit", label: "Fill Down", shortcut: "⌘D", enabled: onGrid, run: () => ctx.fillSelection("down") },
    { id: "edit.fill_right", menu: "Edit", label: "Fill Right", shortcut: "⌘R", enabled: onGrid, run: () => ctx.fillSelection("right") },
    { id: "edit.new_cell", menu: "Edit", label: "New Named Cell…", shortcut: "⌥⌘N", title: "Add a cell with a name instead of a grid address", run: ctx.newCell },

    { id: "cell.edit", menu: "Cell", label: "Edit Cell", shortcut: "↩", enabled: hasSel, run: () => ctx.editSelected() },
    { id: "cell.run", menu: "Cell", label: "Run", shortcut: "⌘↩", title: "Run the selected command, action, or agent", enabled: runnable, run: ctx.runSelected },
    {
      id: "cell.run_selection",
      menu: "Cell",
      label: "Run Selection",
      shortcut: "⇧⌘↩",
      title: "Run every command, action, and agent cell in the selection; verified and running cells are skipped",
      enabled: anyRunnable,
      run: ctx.runSelection,
    },
    {
      id: "cell.driver_answer",
      menu: "Cell",
      label: "Ask the Driver to Answer",
      title: "Have the Driver answer the selected agent cells itself, in its own conversation, rather than running each as an agent of its own (start the Driver first)",
      enabled: () => anyAgent() && ctx.driverRunning(),
      run: ctx.driverAnswer,
    },
    {
      id: "cell.run_column",
      menu: "Cell",
      label: "Run Column",
      shortcut: "⌥⌘↩",
      title: "Run every agent, command, and action cell in this column below the header; verified and running cells are skipped",
      enabled: onGrid,
      run: ctx.runColumn,
    },
    {
      id: "cell.stop",
      menu: "Cell",
      label: "Stop",
      shortcut: "⌘.",
      title: "Stop the run; the cell keeps its last value",
      enabled: () => board.selectedNames().some((n) => {
        const s = board.get(n)?.state;
        return s === "computing" || s === "queued";
      }),
      run: ctx.stopSelected,
    },
    {
      id: "cell.stop_all",
      menu: "Cell",
      label: "Stop All Runs",
      shortcut: "⇧⌘.",
      enabled: () => board.everywhere().some((c) => c.state === "computing" || c.state === "queued"),
      run: ctx.stopAll,
    },
    { id: "cell.budget", menu: "Agent", label: "Set Agent Budget…", title: "Set how much agent runs on this board may use, at API prices, before queued cells wait for you", run: ctx.setBudget },
    { id: "cell.terminal", menu: "Cell", label: "Open Terminal", shortcut: "⌘T", title: "Run the command in a terminal you can type into", enabled: runnable, run: ctx.openTerminalSelected },
    { id: "cell.verify", menu: "Cell", label: "Verify / Unverify", shortcut: "⌘Y", title: "Sign off on the selection, or take a sign-off back; agents can't overwrite what you verified", enabled: verifiable, run: ctx.toggleVerified },
    { id: "cell.flag", menu: "Cell", label: "Flag…", shortcut: "⇧⌘Y", title: "Mark the selection to come back to, with a note", enabled: hasCell, run: ctx.flagSelected },
    {
      id: "cell.next_unverified",
      menu: "Cell",
      label: "Next Unverified Answer",
      shortcut: "⌥⌘↓",
      title: "Jump to the next agent answer nobody has signed off on, in reading order",
      run: ctx.nextUnverified,
    },
    {
      id: "cell.verify_next",
      menu: "Cell",
      label: "Verify and Next",
      shortcut: "⌥⌘Y",
      title: "Sign off on the selection and jump to the next unverified answer",
      enabled: hasCell,
      run: ctx.verifyAndNext,
    },
    {
      id: "cell.review",
      menu: "Cell",
      label: "Review Answers",
      shortcut: "⌥⌘R",
      title: "Check unverified answers one at a time, column by column: V verifies, F flags, J skips, K goes back, Esc stops",
      run: ctx.startReview,
    },
    {
      id: "cell.add_column",
      menu: "Cell",
      label: "Add Question Column…",
      shortcut: "⇧⌘Q",
      title: "Ask a question of every row, about the column you pick (documents, or anything else), and say what kind of answer you want",
      run: ctx.addQuestionColumn,
    },
    {
      id: "cell.answer_type",
      menu: "Cell",
      label: "Answer Type…",
      title: "Say what kind of answer belongs here: yes/no, a date, a number, one of a set; an agent's answer that doesn't fit is sent back",
      enabled: hasSel,
      run: ctx.answerTypeSelection,
    },
    { id: "cell.sparkline", menu: "Cell", label: "Show / Hide History Chart", title: "Draw the cell's past numbers as a small chart on its face", enabled: hasCell, run: ctx.toggleSpark },

    { id: "agent.new", menu: "Agent", label: "New Agent Cell", shortcut: "⇧⌘A", title: "Start an @claude('…') call in the selected cell", enabled: hasSel, run: () => ctx.editSelected("@claude('") },
    { id: "agent.session", menu: "Agent", label: "Open Session", shortcut: "⇧⌘T", title: "Watch and talk to the agent in a terminal beside the grid", enabled: isAgentCell, run: ctx.openSessionSelected },
    {
      id: "agent.notify",
      menu: "Agent",
      label: "Tell Agent About Changed Inputs",
      enabled: () => !!board.active?.stale_inputs?.length,
      run: ctx.notifySelected,
    },
    {
      id: "agent.terminal",
      menu: "Agent",
      label: "Open in Terminal",
      shortcut: "⌥⌘T",
      title: "Continue this cell's agent conversation in a terminal window: joins a live session, or picks up a finished one",
      enabled: () => board.active?.kind === "agent_call" && !!(board.active.session || board.active.session_id),
      run: ctx.handoffSelected,
    },
    { id: "agent.end", menu: "Agent", label: "End Session", enabled: () => !!board.active?.session, run: ctx.endSessionSelected },
    { id: "agent.driver", menu: "Agent", label: "Driver", shortcut: "⇧⌘D", title: "Open the agent that works the board for you: it fills cells, starts other agents, and points you at what needs a look", run: ctx.showDriver },
    { id: "agent.wide_driver", menu: "Agent", label: "Wide Driver", shortcut: "⌥⌘D", title: "Widen the side panel while the Driver tab shows, so its terminal has room; again to narrow it", run: ctx.toggleWideDriver },
    { id: "agent.decisions", menu: "Agent", label: "Needs You", shortcut: "⌘J", title: "See everything waiting on you: questions, suggestions, changed inputs, errors, flags, and what's still working", run: ctx.showDecisions },
    { id: "agent.connect", menu: "Agent", label: "Connect an Agent…", run: () => void chrome.openConnect() },
    { id: "app.updates", menu: "Help", label: "Check for Updates…", title: "See whether a newer recalc is out, and update to it", run: () => void onboard.openUpdates() },
    { id: "app.cli", menu: "Help", label: "Install Command Line Tool…", title: "Put a recalc command on your terminal's PATH", run: () => void onboard.openCli() },

    { id: "view.wrap", menu: "View", label: "Wrap Text", shortcut: "⌥⌘W", title: "Show long values on several lines", run: ctx.toggleWrap },
    { id: "view.freeze", menu: "View", label: "Freeze Panes at Selection", title: "Keep the rows above and the columns left of the selection in view", enabled: onGrid, run: ctx.freeze },
    { id: "view.unfreeze", menu: "View", label: "Unfreeze Panes", run: ctx.unfreeze },
    {
      id: "view.notify",
      menu: "View",
      label: "Notify When Something Needs You",
      title: "Turn desktop notifications on or off for questions, errors, and finished answers that arrive while recalc is in the background",
      run: ctx.toggleNotify,
    },
    {
      id: "view.source",
      menu: "View",
      label: "Show Source",
      shortcut: "⌥⌘S",
      title: "Open the document behind the selected cell, with its cited passage highlighted",
      run: ctx.toggleSource,
    },
    { id: "view.filter_all", menu: "View", label: "Show All Rows", run: () => ctx.setFilter("all") },
    { id: "view.filter_flagged", menu: "View", label: "Show Flagged Rows", run: () => ctx.setFilter("flagged") },
    { id: "view.filter_unverified", menu: "View", label: "Show Unverified Rows", title: "Show only rows with an agent answer nobody has verified yet", run: () => ctx.setFilter("unverified") },
    { id: "view.filter_errors", menu: "View", label: "Show Rows with Errors", run: () => ctx.setFilter("errors") },
    { id: "view.filter_running", menu: "View", label: "Show Running Rows", run: () => ctx.setFilter("running") },
    { id: "view.filter_value", menu: "View", label: "Filter to This Value", title: "Show only the rows whose cell in this column holds the selected cell's value", enabled: hasCell, run: ctx.filterToValue },
    { id: "view.filter_column", menu: "View", label: "Filter Column…", title: "Pick which of this column's values to show; filters on several columns combine", enabled: hasSel, run: ctx.filterColumn },

    { id: "help.palette", menu: "Help", label: "Command Palette", shortcut: "⌘K", run: () => chrome.openPalette(allActions, ctx.boardScripts) },
    { id: "help.cheatsheet", menu: "Help", label: "Cheatsheet", shortcut: "⌘/", run: chrome.openCheatsheet },
    { id: "help.review", menu: "Help", label: "Document Grids Guide", title: "How to review many documents at once: starting, reading, signing off, exporting", run: chrome.openGuide },
    { id: "help.welcome", menu: "Help", label: "Welcome Tour", title: "Tour every cell kind on a live board; on a board with cells, in a new one", run: () => ctx.startNew(() => api.loadTemplate("welcome")) },
    { id: "help.report", menu: "Help", label: "Report a Problem…", title: "Open a new issue on recalc's GitHub page, for a bug or an idea", run: () => void api.openUrl("https://github.com/sshah03/recalc/issues/new/choose").catch((e) => chrome.toast(String(e))) },
  ];
}

/** Push enabled/disabled changes to the native menu, only the deltas. */
export function syncMenu() {
  const diffs: Array<[string, boolean]> = [];
  for (const a of actions) {
    const on = a.enabled ? a.enabled() : true;
    if (enabledSent.get(a.id) !== on) {
      enabledSent.set(a.id, on);
      diffs.push([a.id, on]);
    }
  }
  if (diffs.length) void api.menuEnabled(diffs).catch(() => {});
}

export function resetMenuSync() {
  enabledSent = new Map();
}
