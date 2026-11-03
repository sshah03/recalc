// The IPC surface: one thin wrapper per backend command, typed.

import { board } from "./state.js";
import type { Format, Snapshot } from "./types.js";

interface TauriGlobal {
  core: { invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> };
  event: {
    listen<T>(event: string, cb: (e: { payload: T }) => void): Promise<() => void>;
  };
}

declare global {
  interface Window {
    __TAURI__: TauriGlobal;
  }
}

/** Commands whose `name`, `names`, `at`, `from`, `to` and `origin` are cells:
 * the window names them as the grid shows them (`A2` on the shown sheet,
 * `Sheet1!A2` elsewhere), the backend by full name (`Notes!A2`, `A2`). */
const CELL_COMMANDS = new Set([
  "set_cell", "edit_computed", "delete_cells", "fill", "paste_text", "paste_values", "paste_cells",
  "trigger", "stop", "set_format", "set_answer_type", "run_selection", "resolve_decision", "console",
  "needs_confirm", "set_series", "dismiss_suggestion", "set_mark", "import_csv", "import_files",
  "session_open", "session_end", "session_notify", "session_handoff", "set_cite", "driver_answer",
]);
/** Commands that act on the sheet the grid shows. */
const SHEET_COMMANDS = new Set(["restructure", "sort_rows", "export_csv", "add_question_column"]);

function backendArgs(cmd: string, args?: Record<string, unknown>): Record<string, unknown> | undefined {
  if (SHEET_COMMANDS.has(cmd)) return { ...args, sheet: board.sheet };
  if (!args || !CELL_COMMANDS.has(cmd)) return args;
  const out = { ...args };
  for (const key of ["name", "at", "from", "to", "origin"]) {
    if (typeof out[key] === "string") out[key] = board.full(out[key] as string);
  }
  if (Array.isArray(out.names)) out.names = (out.names as string[]).map((n) => board.full(n));
  return out;
}

const tauri = window.__TAURI__.core;
const invoke = <T>(cmd: string, args?: Record<string, unknown>): Promise<T> => tauri.invoke<T>(cmd, backendArgs(cmd, args));
export const listen = window.__TAURI__.event.listen;

export interface ConsoleInfo {
  command: string;
  cwd: string;
  watch: string | null;
  every: number | null;
}

export interface RunSelection {
  ran: string[];
  /** Left alone: their answers are current. */
  up_to_date: string[];
  need_confirm: string[];
}

export const getBoard = () => invoke<Snapshot>("get_board");
/** A person's write; in a typed cell, what it takes when the value doesn't fit. */
export const setCell = (name: string, content: string) =>
  invoke<string | null>("set_cell", { name, content });
export const editComputed = (name: string, typed: string) =>
  invoke<void>("edit_computed", { name, typed });
export const deleteCells = (names: string[]) => invoke<string[]>("delete_cells", { names });
export const fill = (from: string, to: string, dir: "down" | "right") =>
  invoke<void>("fill", { from, to, dir });
export const pasteText = (at: string, text: string) =>
  invoke<number>("paste_text", { at, text });
/** Paste values as literal cells, whatever they start with; null skips a cell. */
export const pasteValues = (at: string, block: Array<Array<string | null>>) =>
  invoke<number>("paste_values", { at, block });
export const pasteCells = (at: string, origin: string, block: Array<Array<string | null>>) =>
  invoke<number>("paste_cells", { at, origin, block });
export const trigger = (name: string) => invoke<void>("trigger", { name });
export const stop = (name: string) => invoke<boolean>("stop", { name });
export const stopAll = () => invoke<number>("stop_all");
/** Add, rename, move or delete a sheet. */
export const sheetEdit = (action: "add" | "rename" | "move" | "delete" | "duplicate", name: string, to?: string, index?: number) =>
  invoke<void>("sheet_edit", { action, name, to, index });
export const restructure = (axis: "rows" | "cols", at: number, n: number, insert: boolean) =>
  invoke<void>("restructure", { axis, at, n, insert });
export const sortRows = (col: number, first: number, last: number, descending: boolean) =>
  invoke<void>("sort_rows", { col, first, last, descending });
export const setFormat = (names: string[], format: Format | null) =>
  invoke<void>("set_format", { names, format });
export const setAnswerType = (names: string[], kind: string | null, options?: string[], strict?: boolean) =>
  invoke<void>("set_answer_type", { names, kind, options, strict });
export const demos = () => invoke<Array<{ name: string; blurb: string }>>("demos");
export const openDemo = (name: string) => invoke<void>("open_demo", { name });
export const getLayout = () => invoke<string>("get_layout");
export const setLayout = (json: string) => invoke<void>("set_layout", { json });
export const copyText = (text: string) => invoke<void>("copy_text", { text });
export const runSelection = (names: string[], all = false) =>
  invoke<RunSelection>("run_selection", { names, all });
export const setBudget = (dollars: number | null) => invoke<void>("set_budget", { dollars });
export const resolveIndex = (name: string, index: number) =>
  invoke<void>("resolve_decision", { name, index });
export const resolveText = (name: string, text: string) =>
  invoke<void>("resolve_decision", { name, text });
export const console_ = (name: string) => invoke<ConsoleInfo>("console", { name });
export const needsConfirm = (name: string) => invoke<boolean>("needs_confirm", { name });
export const setSeries = (name: string, on: boolean) =>
  invoke<void>("set_series", { name, on });
export const dismissSuggestion = (name: string, value?: string) =>
  invoke<void>("dismiss_suggestion", { name, value: value ?? null });
export const setMark = (names: string[], kind: "verified" | "flagged" | null, note?: string) =>
  invoke<void>("set_mark", { names, kind, note });
export const templates = () => invoke<Array<{ name: string; note: string }>>("templates");
export const loadTemplate = (name: string) => invoke<void>("load_template", { name });
export const menuEnabled = (items: Array<[string, boolean]>) =>
  invoke<void>("menu_enabled", { items });
export interface AgentPath {
  name: string;
  configured: string | null;
  found: string | null;
  /** Its template prints a machine-readable result, so runs carry tokens and cost. */
  reports_usage?: boolean;
}
export const agentPaths = () => invoke<AgentPath[]>("agent_paths");
export const connectCommand = () => invoke<string>("connect_command");
/** Things to tell the person once at launch (running from the disk image). */
export const launchNotices = () => invoke<string[]>("launch_notices");

// boards, undo, import/export
export const undo = () => invoke<string | null>("undo");
export const redo = () => invoke<string | null>("redo");
export const openBoard = (path?: string) => invoke<void>("open_board", { path });
/** Open a board package, an exported board, or cells saved as text; the board's folder, or null if cancelled. */
export const openFile = (path?: string) => invoke<string | null>("open_file", { path });
/** Copy the board somewhere new and switch to the copy; what to tell the person, or null if cancelled. */
export const saveCopy = () => invoke<string | null>("save_copy");
export const renameBoard = (name: string) => invoke<string>("rename_board", { name });
export const moveBoard = () => invoke<string | null>("move_board");
/** Export the board to one `.recalc` file; what was written, or null if cancelled. */
export const exportBoard = () => invoke<string | null>("export_board");
/** The new board's folder, or null when the dialog was cancelled. */
export const newBoard = () => invoke<string | null>("new_board");
export const recentBoards = () => invoke<string[]>("recent_boards");
export const importCsv = (at: string) => invoke<number>("import_csv", { at });
export const importFiles = (at: string) => invoke<number>("import_files", { at });
export const exportCsv = () => invoke<void>("export_csv");
export const exportXlsx = () => invoke<void>("export_xlsx");

export interface Source {
  file: string;
  kind: "text" | "pdf" | "docx";
  text: string;
}
export const readSource = (file: string) => invoke<Source>("read_source", { file });

/** A question column as the library keeps it. */
export interface Column {
  name: string;
  question: string;
  kind: "text" | "yesno" | "date" | "number" | "currency" | "choice" | "quote";
  options?: string[];
  builtin?: boolean;
}
export const columnLibrary = () => invoke<Column[]>("column_library");
export const saveColumn = (column: Column) => invoke<void>("save_column", { column });
export const forgetColumn = (name: string) => invoke<void>("forget_column", { name });
export const addQuestionColumn = (column: Column, agent: string, about = 1, col?: number, model?: string) =>
  invoke<{ header: string; answers: string[]; documents: boolean }>("add_question_column", { column, agent, col, about, model: model || null });
export const modelAliases = () => invoke<string[]>("model_aliases");
export const saveBoardAs = () => invoke<void>("save_board_as");

// live agent sessions
/** Open a cell's session: reattach while it runs, else continue its last
 * conversation; `fresh` starts a new one instead. */
/** Resolves a note for the person when the session isn't the one they'd expect. */
export const sessionOpen = (name: string, cols: number, rows: number, fresh = false) =>
  invoke<string | null>("session_open", { name, cols, rows, fresh });
export const sessionEnd = (name: string) => invoke<void>("session_end", { name });
export const sessionNotify = (name: string) => invoke<void>("session_notify", { name });
/** Open a script a cell runs, in the default text editor (board files only). */
export const openScript = (path: string) => invoke<void>("open_script", { path });
/** Ask the Driver, in its terminal, to answer these agent cells itself. */
export const driverAnswer = (names: string[]) => invoke<void>("driver_answer", { names });
export const notify = (title: string, body: string) => invoke<void>("notify", { title, body });
export const sessionBuffer = (id: string) => invoke<string>("session_buffer", { id });
/** The command that continues a cell's conversation in a terminal; `open`
 * also opens the desktop terminal running it. */
export const sessionHandoff = (name: string, open: boolean) =>
  invoke<string>("session_handoff", { name, open });

// the driver: one agent with a seat in the side panel, no cell
export interface DriverStatus {
  /** attached here / alive in tmux but not shown / not running */
  status: "running" | "detached" | "none";
  agent: string;
  /** Who can take the seat: claude, codex, then the board's [agents]. */
  choices: string[];
  tmux: boolean;
  /** Not running, but Start would continue its last conversation. */
  resumable?: boolean;
}
export const setDriver = (agent: string) => invoke<void>("set_driver", { agent });
export const driverOpen = (cols: number, rows: number, fresh = false) =>
  invoke<void>("driver_open", { cols, rows, fresh });
export const driverEnd = () => invoke<void>("driver_end");
export const driverStatus = () => invoke<DriverStatus>("driver_status");
export const driverHandoff = (open: boolean) => invoke<string>("driver_handoff", { open });

// terminal cells (PTY)
export const ptyOpen = (id: string, command: string, cwd: string, cols: number, rows: number) =>
  invoke<void>("pty_open", { id, command, cwd, cols, rows });
export const ptyWrite = (id: string, data: string) => invoke<void>("pty_write", { id, data });
export const ptyResize = (id: string, cols: number, rows: number) =>
  invoke<void>("pty_resize", { id, cols, rows });
export const ptyClose = (id: string) => invoke<void>("pty_close", { id });

export interface AgentStatus {
  name: string;
  product: string;
  found: string | null;
  /** null when its CLI couldn't say. */
  signed_in: boolean | null;
  install: string;
}
export const agentStatus = () => invoke<AgentStatus[]>("agent_status");

export interface CliStatus {
  exe: string;
  target: string;
  on_path: boolean;
  needs_admin: boolean;
  installed: boolean;
  existing: { path: string; link_to: string | null } | null;
  blocked: string | null;
}
export const cliStatus = () => invoke<CliStatus>("cli_status");
export const installCli = (replace: boolean) => invoke<string>("install_cli", { replace });
export const openUrl = (url: string) => invoke<void>("open_url", { url });

export interface UpdateInfo {
  version: string;
  current: string;
  notes: string;
  date: string | null;
  can_install: boolean;
  why_not: string | null;
  page: string | null;
}
export const updateStatus = () => invoke<{ current: string; auto: boolean; feed: boolean }>("update_status");
export const checkUpdate = () => invoke<UpdateInfo | null>("check_update");
export const setAutoUpdate = (on: boolean) => invoke<void>("set_auto_update", { on });
export const installUpdate = () => invoke<void>("install_update");
