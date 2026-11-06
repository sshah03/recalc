// Starts the app in bridge mode on a fresh board and opens it in WebKit.
// The bridge is desktop/src-tauri/src/bridge.rs.
import { spawn, execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, rmSync, existsSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { webkit } from "playwright";

const here = dirname(fileURLToPath(import.meta.url));
// RECALC_BIN points the checks at another build, e.g. a release one for timing.
export const recalc = process.env.RECALC_BIN ?? join(here, "../../target/debug/recalc");
export const shots = join(here, "shots");
mkdirSync(shots, { recursive: true });

/** A port nothing is listening on. */
async function freePort() {
  const { createServer } = await import("node:net");
  return new Promise((resolve) => {
    const s = createServer();
    s.listen(0, "127.0.0.1", () => {
      const { port } = s.address();
      s.close(() => resolve(port));
    });
  });
}

// Boards are removed when the run exits, not on close: some checks reopen one.
const made = [];
// An app shutting down can still write its run markers as the folder goes,
// so retry, and never fail a finished run over cleanup.
// RECALC_E2E_KEEP=1 keeps them, to look into a run afterwards.
process.once("exit", () => {
  if (process.env.RECALC_E2E_KEEP) return;
  for (const d of made) {
    try {
      rmSync(d, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
    } catch {}
  }
});

/** A board directory: a demo unpacked, or empty. */
export function board(demo) {
  const dir = mkdtempSync(join(tmpdir(), "recalc-e2e-"));
  made.push(dir);
  if (demo) execFileSync(recalc, ["demo", demo, "--into", join(dir, "b"), "--no-open"]);
  else mkdirSync(join(dir, "b"));
  return join(dir, "b");
}

// Paid checks test recalc, not the model: real agents run on efficient
// models, through wrappers recalc starts in place of `claude` and `codex`
// (`[paths]`), so its own commands are intact. Set a variable to "default"
// for the person's own model. pi takes a provider/model pair; RECALC_E2E_PI_DIR
// runs it from another config folder (a local model's provider, say).
const MODELS = {
  claude: process.env.RECALC_E2E_CLAUDE_MODEL ?? "claude-haiku-5-5",
  codex: process.env.RECALC_E2E_CODEX_MODEL ?? "gpt-5.6-luna",
  pi: process.env.RECALC_E2E_PI_MODEL ?? "openai/gpt-5.6-luna",
};
const FLAGS = {
  claude: (m) => `--model '${m}'`,
  codex: (m) => `-c 'model="${m}"' -c 'model_reasoning_effort="low"'`,
  pi: (m) => `--model '${m}' --thinking low`,
};
let wrappers = null;
/** `[paths]` wrappers for the agents found on PATH, made once per run. */
export function agentWrappers() {
  if (wrappers) return wrappers;
  wrappers = {};
  const bin = mkdtempSync(join(tmpdir(), "recalc-e2e-bin-"));
  made.push(bin);
  for (const agent of ["claude", "codex", "pi"]) {
    if (MODELS[agent] === "default") continue;
    let real;
    try {
      real = execFileSync("sh", ["-c", `command -v ${agent}`], { encoding: "utf8" }).trim();
    } catch {
      continue;
    }
    const file = join(bin, agent);
    const flags = FLAGS[agent](MODELS[agent]);
    // Codex drops `-c` given before a subcommand that has `-c` of its own
    // (recalc's exec and resume do): put the flags after the subcommand.
    const script =
      agent === "codex"
        ? `#!/bin/sh\ncase "$1" in\n  exec|e|resume|fork) sub="$1"; shift; exec '${real}' "$sub" ${flags} "$@" ;;\nesac\nexec '${real}' ${flags} "$@"\n`
        : agent === "pi" && process.env.RECALC_E2E_PI_DIR
          ? `#!/bin/sh\nexport PI_CODING_AGENT_DIR='${process.env.RECALC_E2E_PI_DIR}'\nexec '${real}' ${flags} "$@"\n`
          : `#!/bin/sh\nexec '${real}' ${flags} "$@"\n`;
    writeFileSync(file, script, { mode: 0o755 });
    wrappers[agent] = file;
  }
  return wrappers;
}

/** Point a board's real agents at the efficient-model wrappers, leaving any
 * `[paths]` entry a check set itself (stand-ins, missing programs). */
export function useEfficientModels(dir) {
  const file = join(dir, ".recalc/config.toml");
  let text = existsSync(file) ? readFileSync(file, "utf8") : "";
  const has = (agent) => new RegExp(`^\\s*${agent}\\s*=`, "m").test(text.split(/^\[paths\]\s*$/m)[1]?.split(/^\[/m)[0] ?? "");
  const add = Object.entries(agentWrappers())
    .filter(([agent]) => !has(agent))
    .map(([agent, path]) => `${agent} = "${path}"`);
  if (!add.length) return;
  text = /^\[paths\]\s*$/m.test(text)
    ? text.replace(/^\[paths\]\s*$/m, (h) => `${h}\n${add.join("\n")}`)
    : `${text}${text && !text.endsWith("\n") ? "\n" : ""}\n[paths]\n${add.join("\n")}\n`;
  mkdirSync(join(dir, ".recalc"), { recursive: true });
  writeFileSync(file, text);
}

/** The app on `dir`, and a page on it. */
export async function open(dir, { width = 1440, height = 900, dark = false, unset = [], env: extra = {} } = {}) {
  useEfficientModels(dir);
  const port = await freePort();
  // `unset` drops variables to launch as Finder does (no TERM, no locale).
  // New boards go to a folder of the test's own, never ~/Documents.
  const boardsDir = extra.RECALC_BOARDS_DIR ?? mkdtempSync(join(tmpdir(), "recalc-boards-"));
  const env = { ...process.env, RECALC_BOARDS_DIR: boardsDir, ...extra, RECALC_BRIDGE: String(port) };
  for (const k of unset) delete env[k];
  const app = spawn(recalc, [], { cwd: dir, env, stdio: ["ignore", "ignore", "pipe"] });
  let log = "";
  app.stderr.on("data", (d) => (log += d));
  const base = `http://127.0.0.1:${port}`;
  for (let i = 0; ; i++) {
    try {
      if ((await fetch(`${base}/ready`)).ok) break;
    } catch {}
    if (i > 100) {
      app.kill("SIGKILL");
      throw new Error(`the app didn't start:\n${log}`);
    }
    await new Promise((r) => setTimeout(r, 200));
  }
  const browser = await webkit.launch();
  const page = await browser.newPage({ viewport: { width, height }, colorScheme: dark ? "dark" : "light" });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  await page.goto(base);
  await page.waitForSelector("#cells .cell");
  const call = async (cmd, args = {}) => {
    const r = await (await fetch(`${base}/invoke`, { method: "POST", body: JSON.stringify({ cmd, args }) })).json();
    if ("err" in r) throw new Error(`${cmd}: ${r.err}`);
    return r.ok;
  };
  const menu = async (id) => {
    await fetch(`${base}/menu`, { method: "POST", body: JSON.stringify({ id }) });
    await page.waitForTimeout(150);
  };
  const pick = (path) => fetch(`${base}/pick`, { method: "POST", body: JSON.stringify({ path }) });
  /** How many file dialogs the app has asked for so far. */
  const dialogs = async () => (await (await fetch(`${base}/dialogs`)).json()).count;
  let n = 0;
  const shot = async (name) => {
    const file = join(shots, `${String(++n).padStart(2, "0")}-${name}.png`);
    await page.screenshot({ path: file });
    return file;
  };
  const close = async () => {
    await browser.close();
    await fetch(`${base}/quit`, { method: "POST" }).catch(() => {});
    await new Promise((r) => setTimeout(r, 800));
    if (app.exitCode === null) app.kill("SIGKILL");
    endSessions(dir);
  };
  // Kill the app even if the script dies before close().
  process.once("exit", () => {
    if (app.exitCode === null) app.kill("SIGKILL");
    endSessions(dir);
  });
  return { page, dir, call, pick, dialogs, menu, shot, close, errors, log: () => log, proc: app, base, boardsDir };
}

/** Sessions live in tmux and outlast the app; end this board's. */
function endSessions(dir) {
  let panes = "";
  try {
    panes = execFileSync("tmux", ["-L", "recalc", "list-panes", "-a", "-F", "#{session_name}\t#{pane_start_path}\t#{pane_current_path}"], { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] });
  } catch {
    return; // no tmux server: nothing to end
  }
  const real = (p) => p.replace(/^\/private(?=\/var\/)/, "");
  for (const line of panes.split("\n")) {
    const [session, start, current] = line.split("\t");
    if (session && [start, current].some((p) => p && real(p).startsWith(real(dir)))) {
      try {
        execFileSync("tmux", ["-L", "recalc", "kill-session", "-t", session], { stdio: "ignore" });
      } catch {}
    }
  }
}

/** A cell element by name, including the frozen panes. */
export const cell = (page, name) => page.locator(`#cells .cell[data-name='${name}'], #frozen-rows .cell[data-name='${name}'], #frozen-cols .cell[data-name='${name}']`).first();

/** Wait until `f()` is truthy, polling. */
export async function until(f, { timeout = 30000, every = 250, what = "condition" } = {}) {
  const end = Date.now() + timeout;
  for (;;) {
    const v = await f();
    if (v) return v;
    if (Date.now() > end) throw new Error(`timed out waiting for ${what}`);
    await new Promise((r) => setTimeout(r, every));
  }
}
