// Board types mirroring the Rust snapshot in src-tauri/src/board.rs, so a change
// to the serialized Cell surfaces as a type error, not a runtime undefined.

export type StateStr = "ok" | "computing" | "queued" | "stale" | "quarantined" | "awaiting";
export type CellState = StateStr | { error: string };
export type Kind = "literal" | "expr" | "command" | "action" | "agent_call";

/** Explicit-run cells: commands, actions, and agent calls. */
/** The script file a `$` or `!` cell runs (`! python3 scripts/clean.py {B2:B9}`):
 * a relative path with a script's extension, or null. */
export function scriptPath(cell: { kind: string; source?: string | null }): string | null {
  if (cell.kind !== "command" && cell.kind !== "action") return null;
  const body = (cell.source ?? "").replace(/^[$!]\s*/, "").split(" # ")[0];
  for (const tok of body.split(/\s+/)) {
    const t = tok.replace(/^['"]|['"]$/g, "");
    if (!t.startsWith("/") && !t.includes("{") && !t.startsWith("-") && /\.(py|sh|bash|zsh|js|mjs|cjs|ts|rb|pl|r|jl|lua|php|ps1)$/i.test(t)) return t.replace(/^\.\//, "");
  }
  return null;
}

/** A cell's definition rather than a value: `=` a formula, `$ ` a command,
 * `!` an action, `@` an agent. `$` then a number (`$1,200`) is an amount, as
 * src/model.rs `CellKind::of_source` reads it. */
export function isDefinition(text: string): boolean {
  return /^\s*(?:[=!@]|\$(?![-+]?\.?\d))/.test(text);
}

export function isRunnable(kind: Kind): boolean {
  return kind === "command" || kind === "action" || kind === "agent_call";
}

export interface Provenance {
  by: string; // "human" | "engine" | "cli" | "system" | "agent:<id>"
  at: number;
  note?: string;
}

export interface DecisionOption {
  label: string;
  value: string;
  /** What choosing this means, in a few lines: a diff, a sentence. */
  preview?: string;
}

export interface Decision {
  question: string;
  options: DecisionOption[];
  allow_freetext: boolean;
  /** The asker's proposal, if any; never picked automatically. */
  recommended?: number | null;
  asker: string;
  posed_at: number;
  held?: string;
  /** A running agent is waiting on this answer; it goes to the run. */
  from_run?: boolean;
  /** An agent the asker named to answer; the person still can. */
  answerable_by?: string;
}

export interface Suggestion {
  value: string;
  from: string;
  note?: string;
  at: number;
}

export type MarkKind = "verified" | "flagged";

export interface Mark {
  kind: MarkKind;
  note?: string;
  by: string;
  at: number;
}

/** How a cell is shown, never what it holds. Mirrors model::Format. */
export interface Format {
  bold?: boolean;
  italic?: boolean;
  align?: "left" | "center" | "right";
  num?: "text" | "integer" | "number" | "currency" | "percent" | "date";
  fill?: string;
  /** Long text on several lines; the row grows to three. */
  wrap?: boolean;
  /** Digits after the point for number, currency and percent (0–10). */
  decimals?: number;
  /** ISO 4217 code; US dollars when unset. */
  currency?: string;
  /** Fills that follow the value; the first match wins over `fill`. */
  highlight?: Highlight[];
}

/** A fill while the value matches. Mirrors model::Highlight. */
export interface Highlight {
  when: "is" | "contains" | "above" | "below";
  value: string;
  fill: string;
}

/** The passage behind a value: a document in the board and a verbatim
 * quote the source pane finds in it. Mirrors model::Cite. */
export interface Cite {
  file: string;
  quote: string;
  where?: string;
  by: string;
  at: number;
  /** false: the quote isn't in the file as written; absent: not checked. */
  found?: boolean;
}

/** The form answers to a cell must take. Mirrors model::AnswerType. */
export interface AnswerType {
  kind: "text" | "yesno" | "date" | "number" | "currency" | "choice" | "quote" | "pattern" | "json";
  options?: string[];
  /** Refuse a misfit (true) or only note it. */
  strict: boolean;
  allows_not_found?: boolean;
}

/** What the last agent run cost. Mirrors model::Usage. */
export interface Usage {
  input_tokens: number;
  output_tokens: number;
  cost_usd?: number | null;
  turns?: number | null;
  /** The model that did the run (reported, else the one model= asked for). */
  model?: string | null;
}

/** An input a live session was given that has since changed. */
export interface StaleInput {
  name: string;
  was: string;
  now: string;
}

export interface Cell {
  name: string;
  kind: Kind;
  source: string | null;
  value: string;
  deps: string[];
  state: CellState;
  exit_code: number | null;
  stderr: string;
  duration_ms: number | null;
  confirmed: boolean;
  series: boolean;
  def_by: string | null;
  decision: Decision | null;
  pending_suggestions: Suggestion[];
  mark: Mark | null;
  format: Format | null;
  cite: Cite | null;
  answer_type: AnswerType | null;
  usage: Usage | null;
  /** The agent's own conversation id, from its last run or session, so the
   * conversation can be continued in a terminal. */
  session_id: string | null;
  provenance: Provenance;
  // grafted on by the snapshot, or sent only when set
  /** What a streaming agent run is doing now, e.g. "Bash cargo test". */
  activity?: string;
  spark?: number[];
  /** Live session status for @ cells: "running" | "waiting?" (best-effort). */
  session?: string;
  stale_inputs?: StaleInput[];
}

export interface AgentLoad {
  running: number;
  queued: number;
  max: number;
  /** Unix ms until which queued agent cells are held after a usage limit. */
  paused_until?: number | null;
  /** What agent runs may spend before the queue waits; null for no limit. */
  budget?: number | null;
  /** What they have reported spending. */
  spent?: number;
  /** What the Driver's ended sessions used, as Claude Code recorded them. */
  driver_cost?: number;
  driver_tokens?: number;
}

/** What the backend pushes: everything (`full`) or just what moved. */
export interface Snapshot {
  dir: string;
  /** The first sheet's name and the others, in tab order. */
  sheets?: { first: string; others: string[] };
  full: boolean;
  cells: Cell[];
  removed: string[];
  activity: ActivityEntry[];
  agents: AgentLoad;
}

/** A line of the board's activity log, newest first in a snapshot. `by` is
 * who did it, absent for recalc's own notes; `text` starts with `by`. */
export interface ActivityEntry {
  at: number;
  by?: string;
  text: string;
}

export function stateKind(s: CellState): StateStr | "error" {
  return typeof s === "string" ? s : "error";
}

export function errorMsg(s: CellState): string | null {
  return typeof s === "string" ? null : s.error;
}

/** An answer type, said the way the picker and the inspector say it. */
export function answerTypeLabel(t: AnswerType): string {
  const names: Record<AnswerType["kind"], string> = {
    text: "one sentence",
    yesno: "yes / no",
    date: "a date",
    number: "a number",
    currency: "an amount",
    choice: "one of a set",
    quote: "a verbatim quote",
    pattern: "text matching a pattern",
    json: "JSON",
  };
  const base =
    t.kind === "choice" && t.options?.length
      ? `one of ${t.options.join(" / ")}`
      : t.kind === "pattern" && t.options?.length
        ? `text matching ${t.options[0]}`
        : t.kind === "json" && t.options?.length
          ? `JSON with ${t.options.join(", ")}`
          : names[t.kind];
  const noted = t.strict ? base : `${base} (other answers noted, not refused)`;
  return t.allows_not_found === false ? `${noted}; "Not found" doesn't fit` : noted;
}

/** The kind of cell, in a word a person uses. */
export function kindWord(kind: Kind): string {
  return { literal: "value", expr: "formula", command: "command", action: "action", agent_call: "agent" }[kind];
}

/** Where a cell is, in plain words; empty when there is nothing to say. */
export function stateWord(cell: Cell): string {
  switch (stateKind(cell.state)) {
    case "ok":
      return "";
    case "computing":
      return "running";
    case "queued":
      return "queued";
    case "stale":
      return cell.kind === "action" || cell.kind === "agent_call" ? "not run yet" : "out of date";
    case "quarantined":
      return "held: looks destructive";
    case "awaiting":
      return "waiting for your answer";
    case "error":
      return `error: ${errorMsg(cell.state)}`;
  }
}

/** An agent cell that has never run: no answer, session or run on record. */
export function neverRun(c: Cell): boolean {
  return c.kind === "agent_call" && stateKind(c.state) === "stale" && c.value === "" && !c.session && !c.session_id && !c.usage;
}

/** A live session's status, for people. */
export function sessionWord(status: string): string {
  return status === "waiting?" ? "session may need you" : `session ${status}`;
}

/** Tokens the way a status line says them: 1.2k, 40k, 1.3M. */
export function tokensLabel(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 10_000) return `${Math.round(n / 1000)}k`;
  if (n >= 1_000) return `${(n / 1000).toFixed(1)}k`;
  return String(n);
}

/** A cost the way a status line says it: $0.02, $1.40, $12. */
/** What every cost recalc shows is: the agent's own estimate at API prices. */
export const API_PRICES =
  "Priced as the agent reports it, at API prices. On a Claude or ChatGPT plan nothing is billed per run: it's how much of the plan's usage the runs took.";

export function costLabel(usd: number): string {
  if (usd === 0) return "$0";
  if (usd >= 10) return `$${Math.round(usd)}`;
  if (usd >= 1) return `$${usd.toFixed(2)}`;
  return `$${usd.toFixed(usd < 0.01 ? 3 : 2)}`;
}

/** An agent's product name for a person: "claude" is Claude Code. */
export function agentProduct(a: string): string {
  return ({ claude: "Claude Code", codex: "Codex" } as Record<string, string>)[a] ?? a;
}

/** The first line of a value, for a one-line row. */
export function firstLineOf(v: string): string {
  const i = v.indexOf("\n");
  return i === -1 ? v : v.slice(0, i);
}

/** "1m 12s" / "45s" / "2h 05m" for an elapsed span in ms. */
export function elapsedLabel(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${String(s % 60).padStart(2, "0")}s`;
  const h = Math.floor(m / 60);
  return `${h}h ${String(m % 60).padStart(2, "0")}m`;
}

/** An agent's way of saying the document doesn't answer the question. */
export function isNotFound(value: string): boolean {
  const v = value.trim().toLowerCase().replace(/[.!]$/, "");
  return v === "not found" || v === "not stated" || v === "n/a" || v === "none found";
}

export function isAgent(by: string): boolean {
  return by.startsWith("agent:");
}

/** A value an agent produced, for a person to review: an `@` cell's answer,
 * or a value an agent wrote directly (set_cell, set_range). */
export function isAnswer(c: Cell): boolean {
  if (c.value === "") return false;
  if (c.kind === "agent_call") return stateKind(c.state) === "ok";
  return c.kind === "literal" && isAgent(String(c.provenance?.by ?? ""));
}

/** "agent:a1" → "a1"; the human is "you"; recalc's own writes say so. */
export function actorShort(by: string): string {
  if (isAgent(by)) return by.slice(6);
  return { human: "you", cli: "the command line", engine: "recalc", system: "recalc" }[by] ?? by;
}

/** A run's permission request ("The agent wants to use Bash: … Allow it
 * this once?"): what it wants, or null for any other question. */
export function permissionAsk(d: { question: string; options: { label: string }[]; from_run?: boolean }): string | null {
  const m = /^The agent wants to use (.+)\. Allow it this once\?$/s.exec(d.question);
  if (!m || d.options.length !== 3 || d.options[0].label !== "Allow once" || d.options[2].label !== "Deny") return null;
  return m[1];
}

/** What a requested command could touch beyond reading; empty for a read. A hint only. */
export function permissionRisks(what: string, boardDir = ""): string[] {
  const risks: string[] = [];
  if (/\bgit\s+(push|commit|reset|checkout|rebase|merge|tag|clean)\b/.test(what)) risks.push("changes git history");
  if (/(^|[\s;&|:])(curl|wget|ssh|scp|nc|npm\s+(install|i)|pip3?\s+install|brew\s+install|git\s+(clone|fetch|pull))\s/.test(`${what} `)) risks.push("uses the network");
  if (/(^|[\s;&|:])(rm|mv|cp|mkdir|touch|chmod|chown|sudo|dd|tee)\s|>\s*\S|^(Edit|Write|NotebookEdit)\b/.test(what)) risks.push("changes files");
  // A path that isn't under the board's folder (macOS spells /var as
  // /private/var too).
  const real = (p: string) => p.replace(/^\/private(?=\/(var|tmp)\/)/, "");
  const home = real(boardDir);
  const paths = what.match(/(?:^|[\s'"=])((?:\/|~\/|\.\.\/)[^\s'"]*)/g) ?? [];
  if (paths.some((p) => { const x = real(p.replace(/^[\s'"=]/, "")); return !(home && x.startsWith(home)) && !x.startsWith("/dev/null") && !x.startsWith("/tmp/"); })) risks.push("reaches outside the board");
  return risks;
}
