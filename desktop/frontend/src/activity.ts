// The Activity tab: the engine's log as a feed. Each line gets who did it, a
// mark for how a run ended, its cells as links, and when; a burst of the same
// action on many cells reads as one row ("You set 6 cells").

import { icon } from "./icons.js";
import { board } from "./state.js";
import type { ActivityEntry } from "./types.js";
import { isAgent } from "./types.js";

/** One row of the feed: a line, or consecutive alike lines folded together. */
export interface FeedRow {
  at: number;
  by: string | null;
  /** The words after who; `{}` stands for the row's cells when folded. */
  words: string;
  cells: string[];
  mark: "you" | "agent" | "driver" | "ok" | "error" | "ask" | "note";
}

const STATES: Record<string, string> = { ok: "finished", stale: "is out of date", quarantined: "is held for a person", awaiting: "is waiting for your answer", computing: "is running", queued: "is waiting its turn" };
/** Lines within this long of each other fold into one row. */
const FOLD_MS = 2 * 60_000;

/** Who, for display: "You", "Claude", "Codex", "Driver", or the agent's own name. */
export function actorName(by: string): string {
  if (by === "human") return "You";
  if (by === "cli") return "The command line";
  if (by === "engine" || by === "system") return "recalc";
  const id = isAgent(by) ? by.slice(6) : by;
  if (id === "driver") return "Driver";
  if (/^claude/i.test(id)) return "Claude";
  if (/^codex/i.test(id)) return "Codex";
  // A cell's own agent is named after its cell.
  return board.get(id) ? `${id}'s agent` : id;
}

/** Read one log entry: who, what, which cells, and a mark. */
function parse(e: ActivityEntry): FeedRow {
  if (e.by) {
    const words = e.text.startsWith(`${e.by} `) ? e.text.slice(e.by.length + 1) : e.text;
    const mark = e.by === "human" ? "you" : e.by === "agent:driver" ? "driver" : words.startsWith("asks ") ? "ask" : "agent";
    return { at: e.at, by: e.by, words, cells: cellsIn(words), mark };
  }
  const ran = e.text.match(/^engine ran (\S+) -> (error: )?(.*)$/);
  if (ran) {
    const words = ran[2] ? `${ran[1]} failed: ${ran[3]}` : `${ran[1]} ${STATES[ran[3]] ?? ran[3]}`;
    const mark = ran[2] ? "error" : ran[3] === "ok" ? "ok" : ran[3] === "awaiting" ? "ask" : "note";
    return { at: e.at, by: null, words, cells: [ran[1]], mark };
  }
  const words = e.text === "took over as active engine" ? "This window took over the board" : e.text;
  return { at: e.at, by: null, words, cells: cellsIn(words), mark: /: .*(error|failed)|failed/i.test(words) ? "error" : "note" };
}

/** Names in the line that are cells on the board, in order, once each. */
function cellsIn(words: string): string[] {
  const out: string[] = [];
  for (const w of words.split(/[\s,;:()]+/)) {
    const n = w.replace(/'s$/, "");
    if (n && !out.includes(n) && board.get(n)) out.push(n);
  }
  return out;
}

/** The line with its one cell taken out, so lines that differ only by cell match. */
function shape(r: FeedRow): string | null {
  if (r.cells.length !== 1) return null;
  const parts = r.words.split(r.cells[0]);
  return parts.length === 2 ? `${r.by ?? ""}|${r.mark}|${parts[0]}{}${parts[1]}` : null;
}

/** The feed, newest first, with bursts of the same action folded. */
export function feed(entries: ActivityEntry[]): FeedRow[] {
  const out: FeedRow[] = [];
  let last: { key: string; row: FeedRow; oldest: number } | null = null;
  for (const e of entries) {
    const r = parse(e);
    const key = shape(r);
    if (key && last && last.key === key && last.oldest - r.at <= FOLD_MS && !last.row.cells.includes(r.cells[0])) {
      last.row.cells.push(r.cells[0]);
      last.oldest = r.at;
      continue;
    }
    if (key) {
      const parts = r.words.split(r.cells[0]);
      r.words = `${parts[0]}{}${parts[1]}`;
    }
    out.push(r);
    last = key ? { key, row: r, oldest: r.at } : null;
  }
  return out;
}

/** "now", "12m", "14:02", "Yesterday 14:02", "Oct 9, 14:02". */
export function whenLabel(at: number, now = Date.now()): string {
  const ago = now - at;
  if (ago < 60_000) return "now";
  if (ago < 3_600_000) return `${Math.floor(ago / 60_000)}m`;
  const d = new Date(at);
  const time = d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
  const day = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const days = Math.round((day(new Date(now)) - day(d)) / 86_400_000);
  if (days === 0) return time;
  if (days === 1) return `Yesterday ${time}`;
  const date = d.toLocaleDateString(undefined, { month: "short", day: "numeric", ...(d.getFullYear() === new Date(now).getFullYear() ? {} : { year: "numeric" }) });
  return `${date}, ${time}`;
}

/** The full date and time, for the hover. */
export function whenFull(at: number): string {
  return new Date(at).toLocaleString(undefined, { year: "numeric", month: "short", day: "numeric", hour: "numeric", minute: "2-digit", second: "2-digit" });
}

const MARK_TITLE: Record<FeedRow["mark"], string> = { you: "You", agent: "An agent", driver: "The Driver", ok: "Finished", error: "Failed", ask: "A question", note: "recalc" };
/** Cells listed on a folded row before "+N more". */
const SHOWN = 8;

/** One feed row's element; `select` follows a cell link. */
export function feedRow(r: FeedRow, select: (name: string) => void): HTMLElement {
  const row = document.createElement("div");
  row.className = `tray-row act act-${r.mark}`;
  const mark = document.createElement("span");
  mark.className = "glyph";
  mark.title = MARK_TITLE[r.mark];
  if (r.mark === "agent") mark.appendChild(icon("agent", 12));
  else if (r.mark === "driver") mark.appendChild(icon("driver", 12));
  else mark.textContent = { you: "●", ok: "✓", error: "✕", ask: "?", note: "·" }[r.mark];
  const body = document.createElement("div");
  body.className = "act-body";
  const text = document.createElement("div");
  text.className = "act-text";
  if (r.by) {
    const who = document.createElement("span");
    who.className = "act-who";
    who.textContent = actorName(r.by);
    who.title = r.by;
    text.append(who, " ");
  }
  const chip = (name: string) => {
    const c = document.createElement("button");
    c.className = "act-cell";
    c.textContent = name;
    c.title = `Select ${name}`;
    c.onclick = (e) => {
      e.stopPropagation();
      select(name);
    };
    return c;
  };
  const folded = r.words.includes("{}");
  const words = folded ? r.words.replace("{}", r.cells.length > 1 ? `${r.cells.length} cells` : r.cells[0]) : r.words;
  // Cell names in the words become links; a folded row lists its cells below.
  const linked = folded && r.cells.length > 1 ? [] : r.cells;
  const pattern = linked.length ? new RegExp(`(?<![\\w])(${linked.map((n) => n.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join("|")})(?![\\w])`) : null;
  let rest = r.by ? words : words.charAt(0).toUpperCase() + words.slice(1);
  while (pattern) {
    const m = pattern.exec(rest);
    if (!m) break;
    text.append(rest.slice(0, m.index), chip(m[1]));
    rest = rest.slice(m.index + m[1].length);
  }
  text.append(rest);
  body.appendChild(text);
  if (folded && r.cells.length > 1) {
    const list = document.createElement("div");
    list.className = "act-cells";
    const shown = [...r.cells].reverse();
    const fill = (n: number) => {
      list.replaceChildren(...shown.slice(0, n).map(chip));
      if (shown.length > n) {
        const more = document.createElement("button");
        more.className = "act-more";
        more.textContent = `+${shown.length - n} more`;
        more.onclick = (e) => {
          e.stopPropagation();
          fill(shown.length);
        };
        list.appendChild(more);
      }
    };
    fill(SHOWN);
    body.appendChild(list);
  }
  const when = document.createElement("span");
  when.className = "age";
  when.dataset.at = String(r.at);
  when.textContent = whenLabel(r.at);
  when.title = whenFull(r.at);
  row.append(mark, body, when);
  return row;
}

/** Refresh the times shown on the feed. */
export function tickFeed(root: ParentNode) {
  const now = Date.now();
  for (const el of root.querySelectorAll<HTMLElement>(".act .age[data-at]")) el.textContent = whenLabel(Number(el.dataset.at), now);
}
