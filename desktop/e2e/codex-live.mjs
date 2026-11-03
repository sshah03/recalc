// Codex, live: the checks the stand-ins can't settle. Each spends a few
// one-line Codex turns (Codex usage is scarcer than Claude's).
//
//   node codex-live.mjs resume   a Codex cell's session and a Codex driver
//                                continue their conversations after tmux dies
//   node codex-live.mjs elsewhere [codex|claude]
//                                on a computer without the board's
//                                conversations: a recap starts a new one
//   node codex-live.mjs ask [codex|claude]
//                                ask_cell from a live session and a headless
//                                run, left unanswered past Codex's tool timeout
//   node codex-live.mjs hook-setup, then hook
//                                done-means-green's Stop hook holds a Codex
//                                session three times, then flags F2 (Codex
//                                runs a hook only once a person trusts it)
//   node codex-live.mjs sheets   a headless @codex cell reads and writes
//                                another sheet's cells, in a board whose Stop
//                                hook must not trap it
//
// Codex opens on an "Update available" menu whose default runs brew: every
// screen is checked and that menu skipped before anything is typed.

import { mkdtempSync, writeFileSync, mkdirSync, cpSync, readFileSync, realpathSync, existsSync } from "node:fs";
import { join, dirname } from "node:path";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";
import { execFileSync, execSync } from "node:child_process";
import assert from "node:assert/strict";
import { open, until, recalc } from "./lib.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const which = process.argv[2];
const note = (s) => console.log(`• ${new Date().toISOString().slice(11, 19)} ${s}`);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const tmux = (...a) => {
  try {
    return execFileSync("tmux", ["-L", "recalc", ...a], { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] });
  } catch {
    return "";
  }
};
// What's on screen now (menus), and with its scrollback (answers).
// Codex leaves old menus drawn above its prompt: judge by the last lines.
const visible = (s) => tmux("capture-pane", "-p", "-t", s).split("\n").filter((l) => l.trim()).slice(-15).join("\n");
const screen = (s) => tmux("capture-pane", "-p", "-t", s, "-S", "-200");
const cellOf = async (app, n) => (await app.call("get_board")).cells.find((c) => c.name === n);
const st = (c) => (typeof c?.state === "string" ? c.state : JSON.stringify(c?.state));
const dismissWelcome = async (page) => {
  const shown = await page.waitForSelector("#overlay:not([hidden])", { timeout: 4000 }).catch(() => null);
  if (shown) {
    await page.keyboard.press("Escape");
    await page.waitForSelector("#overlay", { state: "hidden", timeout: 5000 });
  }
};

/** Answer Codex's start-up menus in a tmux pane until `done(screen)` holds. */
async function drive(s, done, { timeout = 120000, what = "Codex", scope = "all" } = {}) {
  const start = Date.now();
  let seen = "";
  while (Date.now() - start < timeout) {
    const now = visible(s);
    if (now.trim()) seen = now;
    if (/Update available/i.test(now) && /Skip/i.test(now)) {
      tmux("send-keys", "-t", s, "Down");
      await sleep(400);
      tmux("send-keys", "-t", s, "Enter");
      note(`${s}: skipped Codex's update menu`);
      await sleep(2000);
      continue;
    }
    // Claude Code's trust prompt defaults to "No, exit": move to Yes first.
    if (/Yes, I trust this folder/i.test(now)) {
      // Keys sent before it reads input are dropped, and Enter on the
      // default exits: move until the cursor is on Yes, then confirm.
      for (let i = 0; i < 10 && !/❯\s*Yes, I trust this folder/.test(visible(s)); i++) {
        tmux("send-keys", "-t", s, "Down");
        await sleep(700);
      }
      if (!/❯\s*Yes, I trust this folder/.test(visible(s))) continue;
      tmux("send-keys", "-t", s, "Enter");
      note(`${s}: trusted the folder`);
      // Once: answering again while it redraws would land on "No, exit".
      await until(() => !/Yes, I trust this folder/i.test(visible(s)), { timeout: 20000, what: "the trust prompt to close" }).catch(() => {});
      await sleep(1500);
      continue;
    }
    // A tool permission menu, as a person at the session would: yes, once.
    if (/Do you want to (proceed|make this edit)/i.test(now) && /1\. Yes/.test(now) && !/esc to interrupt/i.test(now)) {
      tmux("send-keys", "-t", s, "Enter");
      note(`${s}: allowed a tool once`);
      await sleep(1500);
      continue;
    }
    if (/Do you trust|trust the (files|contents)|allow Codex to work in this folder/i.test(now) && /1\./.test(now)) {
      tmux("send-keys", "-t", s, "Enter");
      note(`${s}: trusted the folder`);
      await sleep(2000);
      continue;
    }
    const all = scope === "visible" ? now : screen(s);
    if (done(all)) return all;
    await sleep(1500);
  }
  throw new Error(`${what} didn't get there:\n${screen(s) || `(the session is gone; last seen:)\n${seen}`}`);
}

/** Type a line into Codex: pause before Enter, or it takes the keys as a paste. */
async function say(s, text) {
  tmux("send-keys", "-t", s, "-l", text);
  await sleep(1200);
  tmux("send-keys", "-t", s, "Enter");
}

// Codex usage is scarce and these checks test recalc, not the model: every
// Codex here runs the fast, efficient model through a wrapper that recalc
// starts in place of codex (`[paths] codex`), so its own commands are intact.
const CODEX_MODEL = process.env.CODEX_MODEL ?? "gpt-5.6-luna";
const codexWrapper = (() => {
  const real = execSync("command -v codex", { encoding: "utf8" }).trim();
  const file = join(mkdtempSync(join(tmpdir(), "recalc-codex-bin-")), "codex");
  const flags = `-c 'model="${CODEX_MODEL}"' -c 'model_reasoning_effort="low"'`;
  // Codex drops `-c` given before a subcommand that has `-c` of its own
  // (recalc's exec and resume do): put the flags after the subcommand.
  writeFileSync(file, `#!/bin/sh\ncase "$1" in\n  exec|e|resume|fork) sub="$1"; shift; exec '${real}' "$sub" ${flags} "$@" ;;\nesac\nexec '${real}' ${flags} "$@"\n`, { mode: 0o755 });
  return file;
})();

// Claude runs on Haiku the same way, through a `[paths] claude` wrapper.
const CLAUDE_MODEL = process.env.CLAUDE_MODEL ?? "claude-haiku-5-5";
const claudeWrapper = (() => {
  const real = execSync("command -v claude", { encoding: "utf8" }).trim();
  const file = join(mkdtempSync(join(tmpdir(), "recalc-claude-bin-")), "claude");
  writeFileSync(file, `#!/bin/sh\nexec '${real}' --model '${CLAUDE_MODEL}' "$@"\n`, { mode: 0o755 });
  return file;
})();

// What differs between the agents, for the checks that run on both.
const claudeHome = process.env.CLAUDE_CONFIG_DIR ?? join(process.env.HOME, ".claude");
const AGENTS = {
  codex: {
    find: (id) => rolloutOf(id),
    ready: /Ask Codex/,
    bullet: "•",
    model: (text) => text.includes(`"model":"${CODEX_MODEL}"`),
  },
  claude: {
    find: (id) => {
      const out = execSync(`find '${join(claudeHome, "projects")}' -maxdepth 2 -name '${id}.jsonl' 2>/dev/null || true`, { encoding: "utf8" }).trim();
      return out.split("\n").filter(Boolean)[0] ?? null;
    },
    ready: /\? for shortcuts|accept edits|bypass permissions|─\n❯/,
    bullet: "⏺",
    model: (text) => text.includes(`"model":"${CLAUDE_MODEL}`),
  },
};

function board(config = "") {
  const dir = join(mkdtempSync(join(tmpdir(), "recalc-codex-")), "board");
  mkdirSync(join(dir, ".recalc"), { recursive: true });
  writeFileSync(join(dir, ".recalc/config.toml"), `${config}\n[paths]\ncodex = "${codexWrapper}"\nclaude = "${claudeWrapper}"\n`);
  return dir;
}

async function resume() {
  const dir = board('driver = "codex"\n');
  let app = await open(dir);
  let cellSession, driverSession, id, driverId;
  try {
    await dismissWelcome(app.page);
    await app.call("set_cell", { name: "B2", content: "@codex('What is 17 times 3? Reply with only the number.')" });
    const before = tmux("ls", "-F", "#S").split("\n");
    await app.call("session_open", { name: "B2", cols: 120, rows: 40 });
    cellSession = await until(() => tmux("ls", "-F", "#S").split("\n").find((n) => n.endsWith("-B2") && !before.includes(n)), { what: "B2's session" });
    await drive(cellSession, (t) => /\b51\b/.test(t), { what: "B2's answer" });
    id = await until(async () => (await cellOf(app, "B2")).session_id, { timeout: 60000, what: "B2's Codex conversation id" });
    note(`B2 answered 51 in Codex conversation ${id}`);
    await app.call("driver_open", { cols: 120, rows: 40 });
    const boardHash = cellSession.slice(0, -"-B2".length);
    driverSession = await until(() => tmux("ls", "-F", "#S").split("\n").find((n) => n === `${boardHash}-_driver`), { what: "the driver" });
    await drive(driverSession, (t) => /Ask Codex/.test(t) && !/Update available/i.test(t), { what: "the driver's prompt", scope: "visible" });
    await sleep(3000);
    await say(driverSession, "What is 19 times 3? Reply with only the number, and don't change the board.");
    await drive(driverSession, (t) => /\b57\b/.test(t), { what: "the driver's answer" });
    driverId = await until(() => {
      try {
        return readFileSync(join(dir, ".recalc/driver_session"), "utf8").trim() || null;
      } catch {
        return null;
      }
    }, { timeout: 60000, what: "the driver's Codex conversation id" });
    note(`the driver answered 57 in Codex conversation ${driverId}`);
  } finally {
    await app.close();
  }
  // The restart: tmux is gone, the board isn't.
  for (const s of [cellSession, driverSession]) tmux("kill-session", "-t", `=${s}`);
  // Codex takes a moment to let go of a conversation; a reboot gives it longer.
  await sleep(3000);
  assert.notEqual(driverId, id, "the driver recorded its own conversation, not B2's");
  app = await open(dir);
  try {
    await dismissWelcome(app.page);
    assert.equal((await cellOf(app, "B2")).session_id, id, "B2 keeps its conversation id");
    assert.equal((await app.call("driver_status")).resumable, true, "the driver is resumable");
    await app.call("session_open", { name: "B2", cols: 120, rows: 40 });
    const again = await until(() => tmux("ls", "-F", "#S").split("\n").find((n) => n === cellSession), { what: "B2 again" });
    const shown = await drive(again, (t) => /\b51\b/.test(t) || /codex resume/i.test(t), { what: "B2's resumed conversation" });
    note(`B2 after the restart:\n${shown.split("\n").filter((l) => l.trim()).slice(-25).join("\n")}`);
    await drive(again, (t) => /\b51\b/.test(t), { timeout: 30000, what: "B2's earlier answer on screen" });
    assert.equal((await cellOf(app, "B2")).session_id, id, "B2 still has the same id");
    await app.call("driver_open", { cols: 120, rows: 40 });
    const drv = await until(() => tmux("ls", "-F", "#S").split("\n").find((n) => n === driverSession), { what: "the driver again" });
    const dshown = await drive(drv, (t) => /\b57\b/.test(t), { what: "the driver's earlier answer on screen" });
    note(`the driver after the restart:\n${dshown.split("\n").filter((l) => l.trim()).slice(-25).join("\n")}`);
    note("PASS resume: both continued their Codex conversations");
  } finally {
    await app.call("session_end", { name: "B2" }).catch(() => {});
    await app.call("driver_end").catch(() => {});
    await app.close();
  }
}

// Codex runs a project's hooks only once a person has reviewed and trusted
// them, so the hook check runs in one fixed folder: `hook-setup` makes it,
// a person opens Codex there once and trusts the hook, then `hook` runs.
const HOOK_DIR = join(tmpdir(), "recalc-codex-hook", "repo");

async function hookSetup() {
  execSync(`rm -rf '${dirname(HOOK_DIR)}'`);
  mkdirSync(dirname(HOOK_DIR), { recursive: true });
  cpSync(join(here, "../../demos/done-means-green/repo"), HOOK_DIR, { recursive: true });
  mkdirSync(join(HOOK_DIR, ".recalc"), { recursive: true });
  writeFileSync(join(HOOK_DIR, ".recalc/config.toml"), `[paths]\ncodex = "${codexWrapper}"\n`);
  execSync("git init -q && git add -A && git -c user.email=t@t -c user.name=t commit -qm start", { cwd: HOOK_DIR });
  const app = await open(HOOK_DIR);
  try {
    await dismissWelcome(app.page);
    // The template's cells, set straight onto this board.
    const lines = readFileSync(join(here, "../../demos/done-means-green/board.recalctemplate"), "utf8").split("\n");
    const instructions = lines.find((l) => l.startsWith("instructions = ")).slice("instructions = ".length);
    for (const l of lines) {
      const m = /^([A-Za-z_][A-Za-z0-9_]*) = (.*)$/.exec(l);
      if (m && m[1] !== "instructions") await app.call("set_cell", { name: m[1], content: m[2].replace("{instructions}", instructions).replace(/^@claude/, "@codex") });
    }
    await sleep(3000);
  } finally {
    await app.close();
  }
  console.log(`Set up ${HOOK_DIR}. Open Codex there once, trust the folder and review and trust its Stop hook, then quit:\n  cd ${HOOK_DIR} && codex`);
}

async function hook() {
  const dir = HOOK_DIR;
  const app = await open(dir);
  try {
    await dismissWelcome(app.page);
    const path = `${dirname(recalc)}:${process.env.PATH}`;
    note("running: RECALC_AGENT=B2 codex exec 'Reply with only the word done. Do not change any files.'");
    let out = "";
    try {
      out = execSync(
        `RECALC_AGENT=B2 '${codexWrapper}' exec -c 'projects."${realpathSync(dir)}".trust_level="trusted"' "Reply with only the word done. Do not change any files." < /dev/null`,
        { cwd: dir, encoding: "utf8", env: { ...process.env, PATH: path }, timeout: 15 * 60 * 1000, stdio: ["ignore", "pipe", "pipe"] },
      );
    } catch (e) {
      out = `${e.stdout ?? ""}\n${e.stderr ?? ""}`;
    }
    const blocks = (out.match(/acceptance test|STALE|held|failing/gi) ?? []).length;
    note(`codex exec said (tail):\n${out.split("\n").slice(-30).join("\n")}`);
    const [c2, d2, f2] = await Promise.all(["C2", "D2", "F2"].map((n) => cellOf(app, n)));
    note(`C2=${c2?.value} D2=${d2?.value} F2=${f2?.value} (${st(f2)}) block mentions: ${blocks}`);
    assert.ok(c2?.value, "the hook recorded the Codex session id in C2");
    assert.ok(d2?.value, "the hook recorded what it said in D2");
    // Held at "done", Codex either fixes the task (F2 holds) or is flagged after three tries.
    assert.match(String(f2?.value), /^(holds|STALE)$/, "F2 settled");
    note(`PASS hook: Codex's Stop hook reached the board; F2 ${f2?.value}`);
  } finally {
    await app.close();
  }
}

async function sheets() {
  const dir = board();
  // The done-means-green Stop hook, which a headless run must not be held by.
  mkdirSync(join(dir, ".codex"), { recursive: true });
  cpSync(join(here, "../../demos/done-means-green/repo/.codex/hooks.json"), join(dir, ".codex/hooks.json"));
  const app = await open(dir);
  try {
    await dismissWelcome(app.page);
    await app.call("sheet_edit", { action: "add", name: "Notes" });
    await app.call("set_cell", { name: "Notes!A1", content: "the code word is falcon" });
    await app.call("set_cell", {
      name: "B1",
      content: "@codex('Use the recalc tools. Read cell Notes!A1, then set cell Notes!A2 to the single word ok. Reply with only the code word from Notes!A1.')",
    });
    const started = Date.now();
    await app.call("trigger", { name: "B1" });
    await until(async () => !["computing", "queued"].includes(st(await cellOf(app, "B1"))), { timeout: 10 * 60 * 1000, every: 1000, what: "B1 to settle" });
    const [b1, a2] = await Promise.all([cellOf(app, "B1"), cellOf(app, "Notes!A2")]);
    note(`B1=${JSON.stringify(b1?.value)} (${st(b1)}) Notes!A2=${JSON.stringify(a2?.value)} in ${Math.round((Date.now() - started) / 1000)}s`);
    assert.match(String(b1?.value), /falcon/i, "B1 read the other sheet's cell");
    assert.equal(a2?.value, "ok", "B1 wrote the other sheet's cell");
    note("PASS sheets: a headless Codex cell read and wrote Notes!, and the Stop hook let it finish");
  } finally {
    await app.close();
  }
}

// A headless @codex run in the same trusted board isn't held by the Stop
// hook, even on a row whose test fails (RECALC_HEADLESS lets it stop).
async function hookHeadless() {
  const dir = HOOK_DIR;
  const app = await open(dir);
  try {
    await dismissWelcome(app.page);
    await app.call("set_cell", { name: "B5", content: "@codex('Reply with only the word hi. Do not change any files.')" });
    const started = Date.now();
    await app.call("trigger", { name: "B5" });
    await until(async () => !["computing", "queued"].includes(st(await cellOf(app, "B5"))), { timeout: 10 * 60 * 1000, every: 1000, what: "B5 to settle" });
    const b5 = await cellOf(app, "B5");
    const id = b5.session_id;
    const file = id && execSync(`grep -l '${id}' ~/.codex/sessions/*/*/*/rollout-*.jsonl | head -1`, { encoding: "utf8" }).trim();
    const holds = file ? (readFileSync(file, "utf8").match(/<hook_prompt/g) ?? []).length : null;
    note(`B5=${JSON.stringify(b5.value)} (${st(b5)}) in ${Math.round((Date.now() - started) / 1000)}s; Stop hook prompts in its transcript: ${holds}`);
    assert.match(String(b5.value), /\bhi\b/i);
    assert.equal(holds, 0, "the Stop hook didn't hold a headless run");
    assert.ok(!execSync("git diff --name-only -- tally/dates.py", { cwd: dir, encoding: "utf8" }).trim(), "it left dates.py alone");
    note("PASS hook-headless: a headless Codex cell answered without being held");
  } finally {
    await app.close();
  }
}

// The board on a computer that doesn't have its Codex conversations: the
// test's own rollout files are moved out of ~/.codex/sessions (and put back).
const codexHome = process.env.CODEX_HOME ?? join(process.env.HOME, ".codex");
function rolloutOf(id) {
  const out = execSync(`find '${join(codexHome, "sessions")}' -name 'rollout-*-${id}.jsonl' 2>/dev/null || true`, { encoding: "utf8" }).trim();
  return out.split("\n").filter(Boolean)[0] ?? null;
}

async function elsewhere(agent = "codex") {
  const A = AGENTS[agent];
  if (!A) throw new Error(`elsewhere takes codex or claude, not ${agent}`);
  const B = A.bullet;
  const dir = board(`driver = "${agent}"\n`);
  const moved = [];
  const stash = mkdtempSync(join(tmpdir(), "recalc-codex-moved-"));
  const NOTE = `Wrong: it must be odd and above 9000 ("high"), and it can't be 9999.`;
  let app = await open(dir);
  let first, driverId;
  try {
    await dismissWelcome(app.page);
    // ---- this computer: a headless answer, a person's flag, a driver ----
    await app.call("set_cell", { name: "B2", content: `@${agent}('Pick a four-digit number and reply with only that number.')` });
    await app.call("trigger", { name: "B2" });
    await until(async () => !["computing", "queued"].includes(st(await cellOf(app, "B2"))), { timeout: 5 * 60 * 1000, every: 1000, what: "B2's run" });
    first = await cellOf(app, "B2");
    note(`B2 answered ${JSON.stringify(first.value)} (${st(first)}), conversation ${first.session_id}`);
    // Codex writes its conversation file a moment after the run ends.
    const found = await until(() => first.session_id && A.find(first.session_id), { timeout: 20000, every: 1000, what: "the conversation file" }).catch(() => null);
    assert.ok(found, "the headless run's conversation is on this computer");
    await app.call("set_mark", { names: ["B2"], kind: "flagged", note: NOTE });
    const before = tmux("ls", "-F", "#S").split("\n");
    await app.call("driver_open", { cols: 120, rows: 40 });
    const hash = await until(() => tmux("ls", "-F", "#S").split("\n").find((n) => n.endsWith("-_driver") && !before.includes(n)), { what: "the driver" });
    await drive(hash, (t) => A.ready.test(t) && !/Update available|trust this folder/i.test(t), { what: "the driver's prompt", scope: "visible" });
    await sleep(3000);
    await say(hash, "Reply with only the word ready, and don't change the board.");
    await drive(hash, (t) => new RegExp(`${B}\\s*ready\\b`, "i").test(t), { what: "the driver's answer" });
    driverId = await until(() => {
      try {
        return readFileSync(join(dir, ".recalc/driver_session"), "utf8").trim() || null;
      } catch {
        return null;
      }
    }, { timeout: 60000, what: "the driver's id" });
    assert.notEqual(driverId, first.session_id);
    await app.call("driver_end");
  } finally {
    await app.close();
  }
  // ---- another computer: these conversations aren't here ----
  for (const id of [first.session_id, driverId]) {
    const f = A.find(id);
    if (f) {
      execSync(`mv '${f}' '${stash}/'`);
      moved.push([join(stash, f.split("/").pop()), f]);
    }
  }
  note(`moved ${moved.length} conversations out of this computer's ${agent} home`);
  app = await open(dir);
  const found = [];
  const check = (what, ok, detail = "") => {
    found.push({ what, ok });
    note(`${ok ? "ok  " : "FAIL"} ${what}${detail ? `: ${detail}` : ""}`);
  };
  try {
    await dismissWelcome(app.page);
    check("the driver offers Start, not Resume", (await app.call("driver_status")).resumable === false);
    const attach = (() => {
      try {
        return execFileSync(recalc, ["attach", "B2", "--print"], { cwd: dir, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
      } catch (e) {
        return `error: ${(e.stderr ?? e.message).trim()}`;
      }
    })();
    check("recalc attach B2 says the conversation isn't here, rather than resuming nothing", /isn't on this computer|not on this computer/i.test(attach), attach.slice(0, 200));
    const handoff = await app.call("session_handoff", { name: "B2", open: false }).then((r) => `ok: ${JSON.stringify(r)}`, (e) => e.message);
    check("Open in Terminal says why instead of opening a terminal that fails", /isn't on this computer/.test(handoff), handoff.slice(0, 160));
    // Open Session: a new conversation, told what the board kept.
    const opened = await app.call("session_open", { name: "B2", cols: 140, rows: 50 });
    check("Open Session says it's a new conversation", /isn't on this computer, so this is a new one/.test(opened ?? ""), JSON.stringify(opened));
    const s = await until(() => tmux("ls", "-F", "#S").split("\n").find((n) => n.endsWith("-B2")), { what: "B2's session" });
    // The answer is read from the cell: a long recap can scroll off a terminal's screen.
    const answered = await until(async () => {
      const c = await cellOf(app, "B2");
      return /^\d{4}$/.test(c.value) && c.value !== first.value ? c : null;
    }, { timeout: 5 * 60 * 1000, every: 2000, what: "B2's new answer" });
    const shown = tmux("capture-pane", "-p", "-t", s);
    const answer = Number(answered.value);
    check(`${agent} acted on the person's note (odd, above 9000, not 9999)`, answer > 9000 && answer % 2 === 1 && answer !== 9999, String(answer));
    note(`B2's new session:\n${shown.split("\n").filter((l) => l.trim()).slice(-22).join("\n")}`);
    // A live session's answer reaches the cell, and the cell is still the person's prompt.
    const landed = await until(async () => {
      const c = await cellOf(app, "B2");
      return c.value === String(answer) ? c : null;
    }, { timeout: 60000, what: "B2's value to be the new answer" }).catch(async () => cellOf(app, "B2"));
    check("the session's answer is B2's value now", landed.value === String(answer), `${landed.value}`);
    check(`B2 is still its @${agent} prompt`, (landed.source ?? "").startsWith(`@${agent}(`), `${landed.source}`);
    const newId = await until(async () => {
      const id = (await cellOf(app, "B2")).session_id;
      return id && id !== first.session_id ? id : null;
    }, { timeout: 60000, what: "B2's new conversation id" }).catch(() => null);
    check("the new conversation's id is recorded on B2", !!newId && !!A.find(newId), String(newId));
    const convo = newId && A.find(newId) ? readFileSync(A.find(newId), "utf8") : "";
    check("the new conversation starts from the recap, the flag's note intact", convo.includes("You worked on this cell before, on another computer") && convo.includes("it must be odd and above 9000") && convo.includes('(\\"high\\")') && convo.includes("can't be 9999"));
    check(`${agent} ran on the efficient model`, A.model(convo));
    check("the activity log says so", /B2: Its last conversation isn't on this computer/.test((await app.call("get_board")).activity.map((a) => a.text).join("\n")));
    // A new driver, and its own id.
    await app.call("driver_open", { cols: 120, rows: 40 });
    const drv = await until(() => tmux("ls", "-F", "#S").split("\n").find((n) => n === `${s.slice(0, -"-B2".length)}-_driver`), { what: "the driver" });
    const dscreen = await drive(drv, (t) => A.ready.test(t) && !/Update available|trust this folder/i.test(t), { what: "the new driver's prompt", scope: "visible" });
    check("the new driver isn't a resume (no earlier 'ready' on screen)", !new RegExp(`${B}\\s*ready\\b`, "i").test(screen(drv)));
    await sleep(3000);
    await say(drv, "Reply with only the word again, and don't change the board.");
    await drive(drv, (t) => new RegExp(`${B}\\s*again\\b`, "i").test(t), { what: "the new driver's answer" });
    const newDriver = await until(() => {
      const id = readFileSync(join(dir, ".recalc/driver_session"), "utf8").trim();
      return id && id !== driverId ? id : null;
    }, { timeout: 60000, what: "the new driver's id" }).catch(() => null);
    check("the new driver records an id of its own", !!newDriver && newDriver !== newId && !!A.find(newDriver), String(newDriver));
    // A restart on this computer now resumes the new conversations.
    await app.call("session_end", { name: "B2" }).catch(() => {});
    await app.call("driver_end").catch(() => {});
    await app.close();
    await sleep(3000);
    app = await open(dir);
    await dismissWelcome(app.page);
    check("after a restart, the driver offers Resume of its new conversation", (await app.call("driver_status")).resumable === true);
    const again = await app.call("session_open", { name: "B2", cols: 140, rows: 50 });
    check("after a restart, Open Session resumes without the note", !again, JSON.stringify(again));
    const back = await drive(s, (t) => new RegExp(`${B}\\s*${answer}\\b`).test(t), { timeout: 60000, what: "B2's resumed answer" }).catch(() => "");
    check("the resumed conversation shows the answer it gave", !!back);
    check("B2 kept the new id", (await cellOf(app, "B2")).session_id === newId);
  } finally {
    await app.call("session_end", { name: "B2" }).catch(() => {});
    await app.call("driver_end").catch(() => {});
    await app.close();
    for (const [from, to] of moved) execSync(`mv '${from}' '${to}'`);
    note(`put ${moved.length} conversations back`);
  }
  const failed = found.filter((f) => !f.ok);
  note(failed.length ? `${failed.length} of ${found.length} checks failed` : `PASS elsewhere: ${found.length} checks`);
  if (failed.length) process.exitCode = 1;
}

/** Tool results in a Codex or Claude transcript, and the ones that are
 * errors about time running out. */
function toolResults(convo) {
  let results = 0;
  const timedOut = [];
  for (const line of convo.split("\n")) {
    let d;
    try {
      d = JSON.parse(line);
    } catch {
      continue;
    }
    // Codex: *_call_output payloads. Claude: tool_result parts of a user message.
    const outs = [];
    if (/_call_output$/.test(d.payload?.type ?? "")) outs.push(JSON.stringify(d.payload.output));
    for (const part of Array.isArray(d.message?.content) ? d.message.content : []) {
      if (part.type === "tool_result") outs.push((part.is_error ? '"isError":true ' : "") + JSON.stringify(part.content));
    }
    for (const o of outs) {
      results++;
      if (/isError\\?"?:\s*true|is_error/i.test(o) && /timed? ?out|deadline/i.test(o)) timedOut.push(o);
    }
  }
  return { results, timedOut };
}

// Asking the person: ask_cell blocks up to 45 s a call and the agent re-calls
// until answered. Left unanswered past Codex's own tool timeout (60 s), from
// a live session and from a headless run, the answer must still get through.
async function ask(agent = "codex") {
  const A = AGENTS[agent];
  if (!A) throw new Error(`ask takes codex or claude, not ${agent}`);
  const dir = board();
  const WAIT = 110_000;
  const prompt = (cell) =>
    `@${agent}('Use the recalc tool ask_cell on cell ${cell}_choice to ask the person "Which color should the banner be?" with the options Red and Blue. When it returns without an answer, call it again with the same question, and keep going until it is answered. Then answer with only the chosen color.')`;
  const found = [];
  const check = (what, ok, detail = "") => {
    found.push({ what, ok });
    note(`${ok ? "ok  " : "FAIL"} ${what}${detail ? `: ${detail}` : ""}`);
  };
  const app = await open(dir);
  try {
    await dismissWelcome(app.page);
    // ---- a live session ----
    await app.call("set_cell", { name: "B2", content: prompt("B2") });
    const before = tmux("ls", "-F", "#S").split("\n");
    await app.call("session_open", { name: "B2", cols: 140, rows: 50 });
    const s = await until(() => tmux("ls", "-F", "#S").split("\n").find((n) => n.endsWith("-B2") && !before.includes(n)), { what: "B2's session" });
    // Menus answered as a person would, until the question is on the board.
    const asked = Date.now();
    const q = await until(async () => {
      await drive(s, () => true, { timeout: 2000, what: "menus" }).catch(() => {});
      return (await cellOf(app, "B2_choice"))?.decision;
    }, { timeout: 4 * 60 * 1000, every: 2000, what: "B2's question" }).catch(() => null);
    check("the live session's question reaches the board", !!q, q ? `${q.question} [${(q.options ?? []).map((o) => o.label).join(", ")}]` : "");
    if (q) {
      // Unanswered past Codex's tool timeout, watching for the session to give up.
      const until2 = Date.now() + WAIT;
      while (Date.now() < until2) {
        await drive(s, () => true, { timeout: 2000, what: "menus" }).catch(() => {});
        await sleep(3000);
      }
      check("the question is still open after waiting", !!(await cellOf(app, "B2_choice"))?.decision, `${Math.round((Date.now() - asked) / 1000)}s`);
      await app.call("resolve_decision", { name: "B2_choice", index: (q.options ?? []).findIndex((o) => /blue/i.test(o.label)) });
      const shown = await drive(s, (t) => new RegExp(`${A.bullet}\\s*Blue\\b`).test(t), { timeout: 4 * 60 * 1000, what: "B2's answer" }).catch((e) => e.message);
      check("the live session hears the answer and says it", new RegExp(`${A.bullet}\\s*Blue\\b`).test(shown));
      const b2 = await until(async () => {
        const c = await cellOf(app, "B2");
        return /blue/i.test(c.value) ? c : null;
      }, { timeout: 60000, what: "B2's value" }).catch(async () => cellOf(app, "B2"));
      check("its answer lands in B2, prompt kept", /blue/i.test(b2.value) && (b2.source ?? "").startsWith(`@${agent}(`), `${JSON.stringify(b2.value)}`);
      const convo = b2.session_id && A.find(b2.session_id) ? readFileSync(A.find(b2.session_id), "utf8") : "";
      const { results, timedOut } = toolResults(convo);
      check("no tool call timed out while it waited", results > 0 && timedOut.length === 0, `${results} tool results${timedOut.length ? `; ${timedOut[0].slice(0, 160)}` : ""}`);
    }
    await app.call("session_end", { name: "B2" }).catch(() => {});
    // ---- a headless run ----
    await app.call("set_cell", { name: "B3", content: prompt("B3") });
    await app.call("trigger", { name: "B3" });
    const hq = await until(async () => (await cellOf(app, "B3_choice"))?.decision, { timeout: 4 * 60 * 1000, every: 2000, what: "B3's question" }).catch(() => null);
    check("the headless run's question reaches the board", !!hq);
    if (hq) {
      await sleep(WAIT);
      const mid = await cellOf(app, "B3");
      check("the headless run is still waiting after it", !!(await cellOf(app, "B3_choice"))?.decision && /computing/i.test(st(mid)), st(mid));
      await app.call("resolve_decision", { name: "B3_choice", index: (hq.options ?? []).findIndex((o) => /red/i.test(o.label)) });
      await until(async () => !/computing|queued|awaiting/i.test(st(await cellOf(app, "B3"))), { timeout: 5 * 60 * 1000, every: 1000, what: "B3 to settle" }).catch(() => {});
      const b3 = await cellOf(app, "B3");
      check("the headless run finishes with the answer", /^red\.?$/i.test(b3.value.trim()), `${JSON.stringify(b3.value)} (${st(b3)})`);
      const convo = b3.session_id && A.find(b3.session_id) ? readFileSync(A.find(b3.session_id), "utf8") : "";
      check(`${agent} ran on the efficient model`, A.model(convo));
    }
  } finally {
    await app.call("session_end", { name: "B2" }).catch(() => {});
    await app.close();
  }
  const failed = found.filter((f) => !f.ok);
  note(failed.length ? `${failed.length} of ${found.length} checks failed` : `PASS ask: ${found.length} checks`);
  if (failed.length) process.exitCode = 1;
}

// Ask the Driver to Answer: the Driver answers @ cells itself, with
// answer_cell, rather than each running as an agent of its own.
async function driverAnswer(agent = "codex") {
  const A = AGENTS[agent];
  if (!A) throw new Error(`driver-answer takes codex or claude, not ${agent}`);
  const dir = board(`driver = "${agent}"\n`);
  const found = [];
  const check = (what, ok, detail = "") => {
    found.push({ what, ok });
    note(`${ok ? "ok  " : "FAIL"} ${what}${detail ? `: ${detail}` : ""}`);
  };
  const app = await open(dir);
  try {
    await dismissWelcome(app.page);
    const cells = { A1: "City", B1: "Country", A2: "Paris", A3: "Tokyo", A4: "Lima" };
    for (const r of [2, 3, 4]) cells[`B${r}`] = `@${agent}('Which country is {A${r}} in? Answer with only the country name.')`;
    for (const [name, content] of Object.entries(cells)) await app.call("set_cell", { name, content });
    const before = tmux("ls", "-F", "#S").split("\n");
    await app.call("driver_open", { cols: 140, rows: 45 });
    const drv = await until(() => tmux("ls", "-F", "#S").split("\n").find((n) => n.endsWith("-_driver") && !before.includes(n)), { what: "the driver" });
    await drive(drv, (t) => A.ready.test(t) && !/Update available|trust this folder/i.test(t), { what: "the driver's prompt", scope: "visible" });
    await sleep(3000);
    const started = Date.now();
    await app.call("driver_answer", { names: ["B2", "B3", "B4"] });
    const typed = screen(drv);
    check("the request reaches the driver, naming the cells as a range", /Answer these agent cells yourself.*B2:B4/s.test(typed.replace(/\s+/g, " ")));
    const want = { B2: /france/i, B3: /japan/i, B4: /peru/i };
    const done = await until(async () => {
      const b = await app.call("get_board");
      const got = Object.keys(want).map((n) => b.cells.find((c) => c.name === n));
      return got.every((c, i) => c && Object.values(want)[i].test(c.value)) ? got : null;
    }, { timeout: 5 * 60 * 1000, every: 2000, what: "the answers" }).catch(() => null);
    check("the driver answered all three", !!done, done ? done.map((c) => `${c.name}=${c.value}`).join(", ") : "");
    if (done) {
      check("each keeps its prompt", done.every((c) => (c.source ?? "").startsWith(`@${agent}(`)));
      check("each is the driver's answer", done.every((c) => String(c.provenance?.by) === "agent:driver"), done.map((c) => String(c.provenance?.by)).join(", "));
      check("no cell ran as its own agent", done.every((c) => !c.usage && !c.session_id), done.map((c) => `${c.name}:${c.session_id ?? "-"}`).join(" "));
    }
    note(`${Math.round((Date.now() - started) / 1000)}s from asking to answered`);
  } finally {
    await app.call("driver_end").catch(() => {});
    await app.close();
  }
  const failed = found.filter((f) => !f.ok);
  note(failed.length ? `${failed.length} of ${found.length} checks failed` : `PASS driver-answer: ${found.length} checks`);
  if (failed.length) process.exitCode = 1;
}

// Board scripts: asked for a mechanical change, the Driver saves a script and
// an ! cell that runs it (the agents' instructions), rather than one-off
// writes; the person runs it, and it does the job.
async function boardScript(agent = "codex", variant = process.argv[4] ?? "weekly") {
  const A = AGENTS[agent];
  if (!A) throw new Error(`script takes codex or claude, not ${agent}`);
  const dir = board(`driver = "${agent}"\n`);
  const found = [];
  const check = (what, ok, detail = "") => {
    found.push({ what, ok });
    note(`${ok ? "ok  " : "FAIL"} ${what}${detail ? `: ${detail}` : ""}`);
  };
  const app = await open(dir);
  try {
    await dismissWelcome(app.page);
    // weekly: five rows, and they'll come again. many: forty rows, no hint.
    const n = variant === "many" ? 40 : 5;
    const MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    const want = {};
    const cells = { B1: "Signed", C1: "Signed (ISO)" };
    for (let i = 0; i < n; i++) {
      const m = (i % 12) + 1, d = ((i * 7) % 27) + 1, iso = `2026-${String(m).padStart(2, "0")}-${String(d).padStart(2, "0")}`;
      const forms = [iso, `${MONTHS[m - 1].slice(0, 3)} ${d}, 2026`, `${String(m).padStart(2, "0")}/${String(d).padStart(2, "0")}/2026`, `${d} ${MONTHS[m - 1]} 2026`, `2026/${String(m).padStart(2, "0")}/${String(d).padStart(2, "0")}`];
      cells[`B${i + 2}`] = forms[i % forms.length];
      want[`C${i + 2}`] = iso;
    }
    for (const [name, content] of Object.entries(cells)) await app.call("set_cell", { name, content });
    const before = tmux("ls", "-F", "#S").split("\n");
    await app.call("driver_open", { cols: 160, rows: 50 });
    const drv = await until(() => tmux("ls", "-F", "#S").split("\n").find((n) => n.endsWith("-_driver") && !before.includes(n)), { what: "the driver" });
    await drive(drv, (t) => A.ready.test(t) && !/Update available|trust this folder/i.test(t), { what: "the driver's prompt", scope: "visible" });
    await sleep(3000);
    const last = n + 1;
    await say(drv, variant === "weekly"
      ? `Make C2:C${last} the dates from B2:B${last} in ISO format (YYYY-MM-DD). Dates like 03/09/2026 are month/day. We get a new batch of these every week.`
      : `Make C2:C${last} the dates from B2:B${last} in ISO format (YYYY-MM-DD). Dates like 03/09/2026 are month/day.`);
    const action = variant === "once" ? null : await until(async () => {
      const b = await app.call("get_board");
      const a = b.cells.find((c) => c.kind === "action" && /scripts\//.test(c.source ?? ""));
      // Wait for the driver to settle before judging.
      return a && !/esc to interrupt|Working \(|Thinking/i.test(screen(drv)) ? a : null;
    }, { timeout: 6 * 60 * 1000, every: 3000, what: "an ! cell that runs a script" }).catch(() => null);
    if (variant === "once") {
      // Five cells, once: written directly, no script.
      const b = await until(async () => {
        const bb = await app.call("get_board");
        return Object.entries(want).every(([k, v]) => bb.cells.find((c) => c.name === k)?.value === v) ? bb : null;
      }, { timeout: 4 * 60 * 1000, every: 3000, what: "the dates" }).catch(() => null);
      check("a few cells, once: written directly, right", !!b);
      check("and no script for it", !(await app.call("get_board")).cells.some((c) => c.kind === "action"));
    } else
    check(`the driver saved a script and an ! cell that runs it (${variant})`, !!action, action ? `${action.name} = ${action.source}` : `C2=${(await app.call("get_board")).cells.find((c) => c.name === "C2")?.value ?? ""} written directly`);
    if (action && variant !== "once") {
      const script = (action.source.match(/scripts\/[\w.-]+/) ?? [""])[0];
      check("the script is a file in the board", !!script && existsSync(join(dir, script)), script);
      check("the driver left the first run to the person", !action.provenance || String(action.provenance.by) !== "engine" || !action.exit_code, `state ${st(action)}`);
      await app.call("trigger", { name: action.name });
      const ok = await until(async () => {
        const b = await app.call("get_board");
        return Object.entries(want).every(([n, v]) => b.cells.find((c) => c.name === n)?.value === v);
      }, { timeout: 60000, what: "the dates" }).catch(() => false);
      const b = await app.call("get_board");
      const wrong = Object.entries(want).filter(([k, v]) => b.cells.find((c) => c.name === k)?.value !== v).map(([k, v]) => `${k}=${b.cells.find((c) => c.name === k)?.value ?? ""} (want ${v})`);
      check(`running it as the person fills C2:C${last} with ISO dates`, ok, wrong.slice(0, 4).join(" "));
    }
  } finally {
    await app.call("driver_end").catch(() => {});
    await app.close();
  }
  const failed = found.filter((f) => !f.ok);
  note(failed.length ? `${failed.length} of ${found.length} checks failed` : `PASS script: ${found.length} checks`);
  if (failed.length) process.exitCode = 1;
}

const runs = { resume, elsewhere, ask, "driver-answer": driverAnswer, script: boardScript, "hook-setup": hookSetup, hook, "hook-headless": hookHeadless, sheets };
if (!runs[which]) {
  console.error("usage: node codex-live.mjs resume|elsewhere|ask|hook-setup|hook|hook-headless|sheets");
  process.exit(2);
}
await runs[which](process.argv[3]);
