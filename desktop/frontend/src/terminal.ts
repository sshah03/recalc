// Terminals in the inspector: xterm.js views bound to PTYs in the backend.
// Several can be open at once, one per agent session, with one showing;
// the inspector draws the tabs. Two lifetimes:
// - openTerminal spawns a cell's command and kills it when its view closes;
// - attachSession binds to a live agent session (spawning it if needed) and
//   only detaches on close, so the agent keeps running.

import { board } from "./state.js";
import { driverOpen, listen, ptyClose, ptyOpen, ptyResize, ptyWrite, sessionBuffer, sessionOpen } from "./tauri.js";

/** The driver's PTY id: a seat in the side panel, not a cell. It is left
 * out of the inspector's session tabs and of "close every terminal". */
export const DRIVER_ID = "_driver";

// Colors come from the stylesheet tokens, so the terminal follows light and dark.
function theme(): Record<string, string> {
  const css = getComputedStyle(document.documentElement);
  const v = (name: string) => css.getPropertyValue(name).trim();
  return {
    background: v("--term-bg"),
    foreground: v("--text"),
    cursor: v("--accent"),
    cursorAccent: v("--term-bg"),
    selectionBackground: v("--accent-soft"),
    black: v("--text"),
    brightBlack: v("--text-3"),
    red: v("--error"),
    green: v("--ok"),
    yellow: v("--running"),
    blue: v("--dep"),
    magenta: v("--awaiting"),
    cyan: v("--dep"),
    white: v("--surface"),
    brightWhite: v("--canvas"),
  };
}

const scheme = window.matchMedia?.("(prefers-color-scheme: dark)");
scheme?.addEventListener?.("change", () => {
  for (const t of open.values()) t.term.options.theme = theme();
});

interface Open {
  id: string;
  term: Terminal;
  fit: FitAddonInstance;
  container: HTMLElement;
  /** Closing the view kills the PTY (plain terminals) or leaves it (sessions). */
  killOnClose: boolean;
  disposers: Array<() => void>;
}

const open = new Map<string, Open>();
let showing: string | null = null;

/** Keystrokes for PTY `id`, in the order typed. Each write is its own IPC
 * call, and calls can finish out of order, so fast typing came out swapped
 * ("uoy" for "you"): one write is in flight at a time, and what's typed
 * meanwhile goes in the next one. */
function orderedWriter(id: string): (data: string) => void {
  let pending = "";
  let busy = false;
  const flush = async () => {
    busy = true;
    while (pending) {
      const data = pending;
      pending = "";
      // Writes to an exited terminal fail with EIO; ignore them.
      await ptyWrite(id, data).catch(() => {});
    }
    busy = false;
  };
  return (data) => {
    pending += data;
    if (!busy) void flush();
  };
}

async function bind(
  container: HTMLElement,
  id: string,
  killOnClose: boolean,
  sink?: (data: string) => void,
): Promise<Terminal> {
  await closeTerminal(id);
  const term = new Terminal({
    fontFamily: 'ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace',
    fontSize: 12.5,
    lineHeight: 1.25,
    cursorBlink: true,
    theme: theme(),
  });
  const fit = new FitAddon.FitAddon();
  term.loadAddon(fit);
  term.open(container);
  fit.fit();

  const onData = term.onData(orderedWriter(id));
  // Resizes to an exited terminal fail with EIO; ignore them.
  const onResize = term.onResize(({ cols, rows }) => void ptyResize(id, cols, rows).catch(() => {}));
  const write = sink ?? ((data: string) => term.write(data));
  const trust = trustBanner(container, term);
  watchers.set(term, trust);
  const unlistenData = await listen<{ id: string; data: string }>("pty-data", (e) => {
    if (e.payload.id !== id) return;
    write(e.payload.data);
    trust.seen();
  });
  const unlistenExit = await listen<{ id: string }>("pty-exit", (e) => {
    if (e.payload.id === id) term.write("\r\n\x1b[2m[process exited]\x1b[0m\r\n");
  });
  const refit = () => {
    if (showing === id) fit.fit();
  };
  window.addEventListener("resize", refit);
  // Side panel and inspector resize by splitter, not window; observe the container.
  let observer: ResizeObserver | null = null;
  if (typeof ResizeObserver !== "undefined") {
    observer = new ResizeObserver(() => {
      if (!container.hidden && container.isConnected) fit.fit();
    });
    observer.observe(container);
  }

  open.set(id, {
    id,
    term,
    fit,
    container,
    killOnClose,
    disposers: [
      () => onData.dispose(),
      () => onResize.dispose(),
      unlistenData,
      unlistenExit,
      () => trust.remove(),
      () => window.removeEventListener("resize", refit),
      () => observer?.disconnect(),
    ],
  });
  showing = id;
  return term;
}

/** Run `command` in a fresh PTY; the process dies when the view closes. */
export async function openTerminal(container: HTMLElement, id: string, command: string, cwd: string): Promise<void> {
  const term = await bind(container, id, true);
  await ptyOpen(id, command, cwd, term.cols, term.rows);
  term.focus();
}

/** Attach to the cell's live agent session, starting it if needed. Closing
 * the view detaches; the session keeps running. */
export async function attachSession(container: HTMLElement, name: string, fresh = false): Promise<void> {
  // Chunks arriving while the replay is in flight are queued behind it, so
  // reattach output stays ordered. The session's terminal goes by the cell's
  // full name; the calls take the grid's.
  let pending: string[] | null = [];
  const term = await bind(container, board.full(name), false, (data) => {
    if (pending) pending.push(data);
    else term.write(data);
  });
  const note = await sessionOpen(name, term.cols, term.rows, fresh);
  // Said where the person is looking, dimmed, above the agent's own output.
  if (note) term.write(`\x1b[2m${note}\x1b[0m\r\n\r\n`);
  const replay = await sessionBuffer(board.full(name));
  if (replay) {
    term.write(replay);
    watchers.get(term)?.seen();
  }
  for (const d of pending) term.write(d);
  pending = null;
  term.focus();
}

/** Attach to the board's driver, starting it if needed. The driver's view
 * lives in the side panel; closing it detaches like a session. */
export async function attachDriver(container: HTMLElement, fresh = false): Promise<void> {
  let pending: string[] | null = [];
  const term = await bind(container, DRIVER_ID, false, (data) => {
    if (pending) pending.push(data);
    else term.write(data);
  });
  await driverOpen(term.cols, term.rows, fresh);
  const replay = await sessionBuffer(DRIVER_ID);
  if (replay) {
    term.write(replay);
    watchers.get(term)?.seen();
  }
  for (const d of pending) term.write(d);
  pending = null;
  term.focus();
}

/** Bring one open terminal to the front of its holder; the others in that
 * holder stay bound and hidden. Terminals elsewhere (the driver in the side
 * panel, a session in the inspector) are left as they are. */
export function showTerminal(id: string, into: HTMLElement) {
  const t = open.get(id);
  if (!t) return;
  for (const other of open.values()) {
    if (other.container.parentElement === into || other.id === id) other.container.hidden = other.id !== id;
  }
  if (t.container.parentElement !== into) into.appendChild(t.container);
  t.container.hidden = false;
  showing = id;
  t.fit.fit();
  t.term.focus();
}

/** Close one terminal, or every one in the inspector when no id is given;
 * the driver's view is closed only by name. */
export async function closeTerminal(id?: string): Promise<void> {
  const ids = id ? [id] : [...open.keys()].filter((k) => k !== DRIVER_ID);
  for (const one of ids) {
    const t = open.get(one);
    if (!t) continue;
    open.delete(one);
    for (const d of t.disposers) d();
    if (t.killOnClose) await ptyClose(one);
    t.term.dispose();
    t.container.remove();
  }
  if (showing && !open.has(showing)) showing = open.keys().next().value ?? null;
}

/** An inspector terminal is open (the driver's seat doesn't count). */
export function terminalOpen(): boolean {
  return openTerminals().length > 0;
}

export function openTerminals(): string[] {
  return [...open.keys()].filter((k) => k !== DRIVER_ID);
}

export function showingTerminal(): string | null {
  return showing === DRIVER_ID ? null : showing;
}

export function driverViewOpen(): boolean {
  return open.has(DRIVER_ID);
}

const watchers = new WeakMap<Terminal, { seen(): void }>();

/** Claude Code asks whether to trust each new folder (every board is one), and its
 * default answer exits. The banner names the answer that keeps the agent; it never answers. */
function trustBanner(container: HTMLElement, term: Terminal) {
  const banner = document.createElement("div");
  banner.className = "term-banner";
  banner.setAttribute("role", "status");
  banner.hidden = true;
  banner.textContent =
    "Claude Code asks once whether to trust this folder. Press \u2193 to choose \u201cYes, I trust this folder\u201d, then Enter. Its default, No, closes it.";
  container.parentElement?.insertBefore(banner, container);
  // Read the screen as drawn, not the byte stream: a terminal UI under
  // tmux redraws in pieces, cursor jumps even inside a word.
  let pending = 0;
  const check = () => {
    pending = 0;
    const buf = term.buffer.active;
    const rows: string[] = [];
    for (let y = buf.viewportY; y < buf.viewportY + term.rows; y++) rows.push(buf.getLine(y)?.translateToString(true) ?? "");
    banner.hidden = !/trust\s+this\s+folder/i.test(rows.join(" "));
  };
  return {
    seen() {
      if (!pending) pending = window.setTimeout(check, 120);
    },
    remove: () => {
      window.clearTimeout(pending);
      banner.remove();
    },
  };
}
