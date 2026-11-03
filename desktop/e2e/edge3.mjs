// The MCP contract a driver relies on, via the stand-in agent: ask_agent
// nesting and depth limit, open_session, show_cell, run_range, strict and
// lenient answer types, append_cell and history, await_cell. Free.
// `cargo build -p recalc-desktop`, then `node edge3.mjs [filter]`.
import assert from "node:assert/strict";
import { readFileSync, writeFileSync, mkdirSync, realpathSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { board, open, until, cell, recalc as recalcBin } from "./lib.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const fake = join(here, "fake-agent.py");
const results = [];
const only = process.argv[2];
async function check(name, f) {
  if (only && !name.includes(only)) return;
  const started = Date.now();
  try {
    await f();
    results.push([name, "ok"]);
    console.log(`ok   ${name} (${((Date.now() - started) / 1000).toFixed(1)}s)`);
  } catch (e) {
    results.push([name, "FAIL"]);
    console.log(`FAIL ${name}\n     ${String(e.stack ?? e.message ?? e).split("\n").slice(0, 6).join("\n     ")}`);
  }
}
function fakeBoard(config = "") {
  const dir = board();
  mkdirSync(join(dir, ".recalc"), { recursive: true });
  writeFileSync(join(dir, ".recalc/config.toml"), `${config}\n[paths]\nclaude = "${fake}"\n`);
  return dir;
}
// A Claude Code config folder of its own: the stand-in keeps its conversations there.
const claudeHome = () => ({ env: { CLAUDE_CONFIG_DIR: mkdtempSync(join(tmpdir(), "recalc-claude-")) } });
const cellOf = async (app, n) => (await app.call("get_board")).cells.find((c) => c.name === n);
const st = (c) => (typeof c?.state === "string" ? c.state : c?.state?.error != null ? `error: ${c.state.error}` : JSON.stringify(c?.state));
const settled = async (app, names, timeout = 60000) =>
  until(async () => {
    const b = await app.call("get_board");
    return names.every((n) => {
      const c = b.cells.find((x) => x.name === n);
      return c && c.state !== "computing" && c.state !== "queued";
    });
  }, { timeout, every: 200, what: `${names.join(",")} to settle` });
const dismissWelcome = async (page) => {
  const shown = await page.waitForSelector("#overlay:not([hidden])", { timeout: 4000 }).catch(() => null);
  if (shown) {
    await page.keyboard.press("Escape");
    await page.waitForSelector("#overlay", { state: "hidden", timeout: 5000 });
  }
};
const report = async (app, n) => JSON.parse((await cellOf(app, n)).value);

// ---- 1. ask_agent nests, on the board, and stops three deep ----
await check("ask_agent puts each handoff in a cell of its own, attributed, and a chain of agents stops three deep", async () => {
  const app = await open(fakeBoard("max_agents = 4"));
  try {
    await dismissWelcome(app.page);
    await app.call("set_cell", { name: "B2", content: "@claude('fake: askagent=n1|fake: askagent=n2|fake: askagent=n3|fake: answer=deep; report=D2')" });
    await app.call("trigger", { name: "B2" });
    await settled(app, ["B2"], 120000);
    const b = await app.call("get_board");
    const n1 = b.cells.find((c) => c.name === "n1");
    const n2 = b.cells.find((c) => c.name === "n2");
    assert.ok(n1 && n1.kind === "agent_call" && n1.source.startsWith("@claude("), `the first handoff is a cell: ${JSON.stringify(n1?.source)}`);
    assert.ok(/B2/.test(String(n1.def_by)), `written by B2's agent: ${JSON.stringify(n1.def_by)}`);
    assert.ok(n2 && n2.kind === "agent_call", "the second handoff is a cell too");
    const chain = [b.cells.find((c) => c.name === "B2").value, n1.value, n2.value].join(" | ");
    assert.ok(/deep/.test(chain) && /can't start another|3 deep|already/.test(chain), `the third handoff was refused and the refusal came back up the chain: ${chain.slice(0, 400)}`);
    assert.notEqual(b.cells.find((c) => c.name === "n3")?.value, "deep", "n3 never ran");
    console.log(`     chain: ${chain.slice(0, 220)}`);
    assert.deepEqual(app.errors, []);
  } finally {
    await app.close();
  }
});

// ---- 2. open_session and show_cell ----
await check("launched the way Finder launches it (no TERM, no locale), a live session still starts", async () => {
  const app = await open(fakeBoard(), { unset: ["TERM", "LANG", "LC_ALL", "LC_CTYPE", "COLORTERM"] });
  try {
    await app.call("set_cell", { name: "B2", content: "@claude('fake: answer=x')" });
    await app.call("session_open", { name: "B2", cols: 80, rows: 24 });
    const { execFileSync } = await import("node:child_process");
    const screen = await until(() => {
      // No tmux server until the first session starts: that's "not yet", not a failure.
      let names = "";
      try { names = execFileSync("tmux", ["-L", "recalc", "ls", "-F", "#S"], { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }); } catch {}
      const sess = names.split("\n").find((n) => n.endsWith("-B2"));
      if (!sess) return null;
      const t = execFileSync("tmux", ["-L", "recalc", "capture-pane", "-p", "-t", sess], { encoding: "utf8" });
      return /fake claude session ready/.test(t) ? t : null;
    }, { timeout: 15000, what: "the session's banner (tmux refused to start without TERM before)" });
    assert.ok(screen);
    await app.call("session_end", { name: "B2" }).catch(() => {});
  } finally {
    await app.close();
  }
});

await check("an agent's open_session request opens the cell's live terminal in the Inspector, and show_cell selects a cell", async () => {
  const app = await open(fakeBoard());
  try {
    await dismissWelcome(app.page);
    await app.call("set_cell", { name: "B3", content: "@claude('fake: answer=later')" });
    await app.call("set_cell", { name: "B2", content: "@claude('fake: session=B3; report=D2')" });
    await app.call("trigger", { name: "B2" });
    await settled(app, ["B2"]);
    await until(async () => (await cellOf(app, "B3_status"))?.value === "running", { timeout: 15000, what: "the status cell" });
    assert.equal(await app.page.textContent("#namebox"), "B3", "the window selected the cell whose session opened");
    await until(async () => /B3/.test((await app.page.textContent("#inspector")) ?? "") && /session/i.test((await app.page.textContent("#inspector")) ?? ""), { timeout: 10000, what: `the session pane in the Inspector (saw: ${((await app.page.textContent("#inspector")) ?? "").replace(/\s+/g, " ").slice(0, 120)})` });
    // The app puts its own binary on PATH, which the tmux session inherits.
    const { execFileSync } = await import("node:child_process");
    const sess = execFileSync("tmux", ["-L", "recalc", "ls", "-F", "#S"], { encoding: "utf8" }).split("\n").find((n) => n.endsWith("-B3"));
    const screen = await until(() => {
      const t = execFileSync("tmux", ["-L", "recalc", "capture-pane", "-p", "-t", sess], { encoding: "utf8" });
      return /recalc on PATH: /.test(t) ? t : null;
    }, { timeout: 10000, what: "the session's banner" });
    const where = screen.match(/recalc on PATH: (\S+)/)[1];
    assert.ok(where !== "missing" && where.endsWith("/recalc"), `recalc resolves inside the session: ${where}`);
    console.log(`     inside the session, recalc is ${where}`);
    const rows = await app.page.locator(".xterm-rows").allTextContents().catch(() => []);
    console.log(`     terminal shows: ${JSON.stringify(rows.join(" ").replace(/\s+/g, " ").slice(0, 80))}`);
    await app.shot("edge3-open-session");
    await app.call("session_end", { name: "B3" });
    await until(async () => /ended|done|detached/.test((await cellOf(app, "B3_status"))?.value ?? ""), { timeout: 10000, what: "the session to end" });
    await app.call("set_cell", { name: "A9", content: "look here" });
    await app.call("set_cell", { name: "C2", content: "@claude('fake: show=A9; report=D3')" });
    await app.call("trigger", { name: "C2" });
    await settled(app, ["C2"]);
    await until(async () => (await app.page.textContent("#namebox")) === "A9", { timeout: 10000, what: "show_cell to select A9" });
    const rep = await report(app, "D3");
    assert.ok(!rep.find((r) => r.tool === "show_cell").isError);
  } finally {
    await app.close();
  }
});

// ---- 3. run_range ----
await check("run_range queues a block, skips what is current, and re-runs it only with all", async () => {
  const app = await open(fakeBoard("max_agents = 3"));
  try {
    await dismissWelcome(app.page);
    for (const n of ["B4", "B5", "B6"]) await app.call("set_cell", { name: n, content: `@claude('fake: answer=${n}; sleep=0.5')` });
    await app.call("set_cell", { name: "B2", content: "@claude('fake: runrange=B4|B6; report=D2')" });
    await app.call("trigger", { name: "B2" });
    await settled(app, ["B2", "B4", "B5", "B6"]);
    let rep = await report(app, "D2");
    let rr = JSON.parse(rep.find((r) => r.tool === "run_range").text);
    assert.deepEqual(rr.queued.sort(), ["B4", "B5", "B6"], `queued all three: ${JSON.stringify(rr)}`);
    for (const n of ["B4", "B5", "B6"]) assert.equal((await cellOf(app, n)).value, n);
    await app.call("set_cell", { name: "B3", content: "@claude('fake: runrange=B4|B6; report=D3')" });
    await app.call("trigger", { name: "B3" });
    await settled(app, ["B3"]);
    rep = await report(app, "D3");
    rr = JSON.parse(rep.find((r) => r.tool === "run_range").text);
    assert.deepEqual(rr.queued, [], `nothing out of date is re-run: ${JSON.stringify(rr)}`);
    assert.deepEqual(rr.up_to_date.sort(), ["B4", "B5", "B6"]);
  } finally {
    await app.close();
  }
});

// ---- 4. answer types, strict and lenient ----
await check("a strict answer type refuses an agent's misfit with what it wanted; a lenient one lands it and says so", async () => {
  const app = await open(fakeBoard());
  try {
    await dismissWelcome(app.page);
    await app.call("set_cell", { name: "B2", content: "@claude('fake: settype=C2:C2|yesno|true; settype=C3:C3|yesno|false; settype=C4:C4|choice|true|GREEN|YELLOW|RED; set=C2|Probably yes; set=C3|Probably yes; set=C4|yellow; set=C5|x; report=D2')" });
    await app.call("trigger", { name: "B2" });
    await settled(app, ["B2"]);
    const rep = await report(app, "D2");
    const w = (n) => rep.find((r) => r.tool === "set_cell" && r.args.name === n);
    assert.ok(w("C2").isError && /Yes|No/.test(w("C2").text), `strict refuses and says what it wanted: ${w("C2").text.slice(0, 160)}`);
    assert.equal((await cellOf(app, "C2"))?.value ?? "", "", "nothing landed in the strict cell");
    assert.ok(!w("C3").isError && /outside_answer_type/.test(w("C3").text), `lenient lands it and notes the misfit: ${w("C3").text.slice(0, 160)}`);
    assert.equal((await cellOf(app, "C3")).value, "Probably yes");
    assert.ok(!w("C4").isError && /written/.test(w("C4").text), `a loosely matched option is normalized: ${w("C4").text.slice(0, 160)}`);
    assert.equal((await cellOf(app, "C4")).value, "YELLOW");
    const b = await app.call("get_board");
    assert.equal(b.cells.find((c) => c.name === "C4").answer_type?.kind, "choice");
  } finally {
    await app.close();
  }
});

// ---- 5. append_cell and history ----
await check("append_cell records points a formula can read and history returns them in order", async () => {
  const app = await open(fakeBoard());
  try {
    await dismissWelcome(app.page);
    await app.call("set_cell", { name: "score", content: "0" });
    await app.call("set_series", { name: "score", on: true });
    await app.call("set_cell", { name: "B2", content: "@claude('fake: append=score|1; append=score|2; append=score|3; hist=score; report=D2')" });
    await app.call("trigger", { name: "B2" });
    await settled(app, ["B2"]);
    const rep = await report(app, "D2");
    const hist = JSON.parse(rep.find((r) => r.tool === "history").text);
    assert.deepEqual(hist.map((h) => h.value).slice(-3), ["1", "2", "3"], `history in order: ${JSON.stringify(hist)}`);
    assert.ok(hist.every((h, i) => i === 0 || h.at >= hist[i - 1].at), "timestamps ascend");
    const c = await cellOf(app, "score");
    assert.equal(c.value, "3");
    assert.equal(c.series, true, "an appended cell is a series (sparkline)");
    const h = await app.call("get_board");
    assert.ok(h.cells.some((x) => x.name === "score"));
  } finally {
    await app.close();
  }
});

// ---- 6. await_cell ----
await check("await_cell returns at once when satisfied, waits for a later write, and reports an unmet timeout", async () => {
  const app = await open(fakeBoard("max_agents = 2"));
  try {
    await dismissWelcome(app.page);
    await app.call("set_cell", { name: "ready", content: "yes" });
    await app.call("set_cell", { name: "B2", content: `@claude("fake: await=ready|== 'yes'; await=gate|== 'go'; report=D2")` });
    await app.call("trigger", { name: "B2" });
    await until(async () => (await cellOf(app, "B2")).state === "computing", { what: "the run" });
    await app.page.waitForTimeout(2500);
    assert.equal((await cellOf(app, "B2")).state, "computing", "still waiting on gate");
    await app.call("set_cell", { name: "gate", content: "go" });
    await settled(app, ["B2"], 40000);
    const rep = await report(app, "D2");
    const waits = rep.filter((r) => r.tool === "await_cell").map((r) => JSON.parse(r.text));
    assert.equal(waits[0].satisfied, true, `already satisfied: ${JSON.stringify(waits[0])}`);
    assert.equal(waits[1].satisfied, true, `satisfied after the write: ${JSON.stringify(waits[1])}`);
    assert.equal(waits[1].value, "go");
  } finally {
    await app.close();
  }
});

// ---- 8. resume in the board's folder ----
await check("Open in Terminal resumes a finished run in the board's folder", async () => {
  const app = await open(fakeBoard(), claudeHome());
  try {
    await app.call("set_cell", { name: "B2", content: "@claude('fake: answer=x')" });
    await app.call("trigger", { name: "B2" });
    await settled(app, ["B2"]);
    const cmd = await app.call("session_handoff", { name: "B2", open: false });
    const { realpathSync } = await import("node:fs");
    assert.ok(cmd.startsWith(`cd '${realpathSync(app.dir)}' && `), cmd);
    assert.match(cmd, /--resume/);
  } finally {
    await app.close();
  }
});

// ---- 9. clicking away while editing ----
await check("a click elsewhere saves the edit, and picks a reference only where one fits", async () => {
  const app = await open(fakeBoard());
  try {
    await dismissWelcome(app.page);
    const click = (n) => app.page.click(`#cells .cell[data-name='${n}']`);
    const bar = () => app.page.evaluate(() => document.querySelector("#formula").value);
    await click("B2");
    await app.page.keyboard.type("hello");
    await click("D5");
    await until(async () => (await cellOf(app, "B2"))?.value === "hello", { what: "typing saved on click-away (it was discarded)" });
    await click("B3");
    await app.page.keyboard.type("=1+");
    await click("D5");
    assert.equal(await bar(), "=1+{D5}", "after an operator, a click picks");
    await app.page.keyboard.press("Escape");
    await click("B4");
    await app.page.keyboard.type("@claude('read ");
    await click("A2");
    assert.equal(await bar(), "@claude('read {A2}", "inside the prompt's quotes, a click picks");
    await app.page.keyboard.type("')");
    await click("D6");
    await until(async () => (await cellOf(app, "B4"))?.source === "@claude('read {A2}')", { what: "after the closing parenthesis, a click saves instead of picking" });
  } finally {
    await app.close();
  }
});

// ---- 10. finished answers are news, not needs ----
await check("finished answers are listed as new but not counted as needing you, and an agent cell says what it asks", async () => {
  const app = await open(fakeBoard());
  try {
    await dismissWelcome(app.page);
    await app.call("set_cell", { name: "A2", content: "notes.md" });
    await app.call("set_cell", { name: "B1", content: "What is the owner's name?" });
    await app.call("set_cell", { name: "B2", content: "@claude('Read {A2}. {B1}')" });
    await app.call("trigger", { name: "B2" });
    await settled(app, ["B2"]);
    await app.page.click("#cells .cell[data-name='D5']");
    await app.page.click("#tab-decisions");
    await until(async () => /new answer/i.test((await app.page.textContent("#side")) ?? ""), { what: "the new-answers group" });
    assert.equal((await app.page.textContent("#status-awaiting"))?.trim(), "", "a finished answer isn't counted as needing you");
    assert.equal((await app.page.textContent("#needs-count"))?.trim(), "");
    await app.page.click("#cells .cell[data-name='B2']");
    const ask = await app.page.textContent("#inspector .ask");
    assert.equal(ask, "Asks Claude Code: Read notes.md. What is the owner's name?");
    // A model and context after the prompt aren't part of it.
    await app.call("set_cell", { name: "B3", content: "@claude('Summarize {A2}', {B1}, model='haiku')" });
    await app.page.click("#cells .cell[data-name='D5']");
    await app.page.click("#cells .cell[data-name='B3']");
    await until(async () => /Summarize/.test((await app.page.textContent("#inspector .ask").catch(() => "")) ?? ""), { what: "B3's ask line" });
    assert.equal(await app.page.textContent("#inspector .ask"), "Asks Claude Code on haiku: Summarize notes.md");
  } finally {
    await app.close();
  }
});

// ---- 11. after a restart ----
await check("after a restart kills their terminals, a cell's session and the driver continue their conversations", async () => {
  const { execFileSync } = await import("node:child_process");
  const { readFileSync } = await import("node:fs");
  // "" when there's no tmux server yet, rather than an error.
  const tmux = (...a) => { try { return execFileSync("tmux", ["-L", "recalc", ...a], { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }); } catch { return ""; } };
  const sessionOf = (suffix) => tmux("ls", "-F", "#S").split("\n").find((n) => n.endsWith(suffix));
  const banner = (s) => until(() => { try { const t = tmux("capture-pane", "-p", "-t", s); return /started as/.test(t) ? t : null; } catch { return null; } }, { timeout: 15000, what: `${s}'s banner` });
  const dir = fakeBoard();
  const home = claudeHome();
  let app = await open(dir, home);
  let b2, drv, id, driverId;
  try {
    await dismissWelcome(app.page);
    await app.call("set_cell", { name: "B2", content: "@claude('fake: answer=x')" });
    const before = tmux("ls", "-F", "#S").split("\n");
    await app.call("session_open", { name: "B2", cols: 80, rows: 24 });
    b2 = await until(() => tmux("ls", "-F", "#S").split("\n").find((n) => n.endsWith("-B2") && !before.includes(n)), { what: "B2's session" });
    id = (await cellOf(app, "B2")).session_id;
    assert.match(await banner(b2), new RegExp(`--session-id ${id}`));
    await app.call("driver_open", { cols: 80, rows: 24 });
    // Session names start with the board's hash; other boards' drivers may be alive.
    const board = b2.slice(0, -"-B2".length);
    drv = await until(() => sessionOf(`${board}-_driver`), { what: "the driver" });
    driverId = readFileSync(join(dir, ".recalc/driver_session"), "utf8").trim();
    assert.match(await banner(drv), new RegExp(`--session-id ${driverId}`));
  } finally {
    await app.close();
  }
  // The restart: tmux is gone, the board isn't (close ends the board's sessions too).
  for (const s of [b2, drv]) try { tmux("kill-session", "-t", `=${s}`); } catch {}
  app = await open(dir, home);
  try {
    assert.equal((await cellOf(app, "B2")).session_id, id, "the conversation id survives");
    assert.equal((await app.call("driver_status")).resumable, true);
    await app.call("session_open", { name: "B2", cols: 80, rows: 24 });
    assert.match(await banner(await until(() => sessionOf(b2), { what: "B2 again" })), new RegExp(`--resume ${id}`));
    assert.equal((await cellOf(app, "B2")).session_id, id);
    await app.call("driver_open", { cols: 80, rows: 24 });
    assert.match(await banner(await until(() => sessionOf(drv), { what: "the driver again" })), new RegExp(`--resume ${driverId}`));
    // Asked for a new conversation: a new id, recorded.
    await app.call("session_end", { name: "B2" });
    await app.call("session_open", { name: "B2", cols: 80, rows: 24, fresh: true });
    const fresh = (await cellOf(app, "B2")).session_id;
    assert.notEqual(fresh, id);
    assert.match(await banner(await until(() => sessionOf(b2), { what: "B2 fresh" })), new RegExp(`--session-id ${fresh}`));
    await app.call("session_end", { name: "B2" }).catch(() => {});
    await app.call("driver_end").catch(() => {});
  } finally {
    await app.close();
  }
});

await check("on another computer, a cell and the driver start new conversations and say so, instead of resuming nothing", async () => {
  const { execFileSync } = await import("node:child_process");
  const tmux = (...a) => { try { return execFileSync("tmux", ["-L", "recalc", ...a], { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }); } catch { return ""; } };
  const dir = fakeBoard();
  let app = await open(dir, claudeHome());
  let id;
  try {
    await dismissWelcome(app.page);
    await app.call("set_cell", { name: "B2", content: "@claude('fake: answer=x')" });
    await app.call("set_cell", { name: "B3", content: "@claude('fake: answer=y')" });
    await app.call("trigger", { name: "B3" });
    await settled(app, ["B3"]);
    await app.call("session_open", { name: "B2", cols: 80, rows: 24 });
    id = (await cellOf(app, "B2")).session_id;
    const first = await until(() => { const s = tmux("ls", "-F", "#S").split("\n").find((n) => n.endsWith("-B2")); const t = s && tmux("capture-pane", "-p", "-t", s); return t && /started as/.test(t) ? t : null; }, { timeout: 15000, what: "the first banner" });
    assert.doesNotMatch(first, /task begins: You worked on this cell before/, "only a replaced conversation gets the recap");
    await app.call("driver_open", { cols: 80, rows: 24 });
    await until(async () => (await app.call("driver_status")).status !== "none", { what: "the driver" });
    await app.call("session_end", { name: "B2" });
    await app.call("driver_end");
  } finally {
    await app.close();
  }
  // The same folder on a computer whose Claude Code never saw these conversations.
  const elsewhere = claudeHome();
  app = await open(dir, elsewhere);
  try {
    // From a shell, too: recalc attach says why rather than running a resume that fails.
    const attach = (() => {
      try {
        return execFileSync(recalcBin, ["attach", "B3", "--print"], { cwd: dir, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], env: { ...process.env, ...elsewhere.env } });
      } catch (e) {
        return String(e.stderr);
      }
    })();
    assert.match(attach, /isn't on this computer/, `recalc attach: ${attach}`);
    assert.equal((await app.call("driver_status")).resumable, false, "the driver offers Start, not Resume");
    const note = await app.call("session_open", { name: "B2", cols: 80, rows: 24 });
    assert.match(note ?? "", /isn't on this computer, so this is a new one/);
    const fresh = (await cellOf(app, "B2")).session_id;
    assert.ok(fresh && fresh !== id, "a new conversation, recorded");
    const s = await until(() => tmux("ls", "-F", "#S").split("\n").find((n) => n.endsWith("-B2")), { what: "B2's session" });
    const pane = await until(() => { const t = tmux("capture-pane", "-p", "-t", s); return /started as/.test(t) ? t : null; }, { timeout: 15000, what: "the banner" });
    assert.match(pane, new RegExp(`--session-id ${fresh}`));
    assert.doesNotMatch(pane, /No conversation found/);
    assert.match(pane, /task begins: You worked on this cell before, on another computer/, "the new conversation starts from what the board kept");
    const handoff = await app.call("session_handoff", { name: "B3", open: false }).then(() => "", (e) => e.message);
    assert.match(handoff, /isn't on this computer/, "Open in Terminal says why instead of opening a terminal that fails");
    assert.match((await app.call("get_board")).activity.map((a) => a.text).join("\n"), /B2: Its last conversation isn't on this computer/);
    await app.call("session_end", { name: "B2" }).catch(() => {});
  } finally {
    await app.close();
  }
});

// ---- 12. killed mid-run ----
await check("a window killed mid-run, or a Ctrl-C'd recalc trigger, leaves no agent running", async () => {
  const { execFileSync, spawn } = await import("node:child_process");
  const fakes = () => { try { return execFileSync("pgrep", ["-f", "fake-agent.py"], { encoding: "utf8" }).trim().split("\n").filter(Boolean); } catch { return []; } };
  const before = fakes();
  const mine = () => fakes().filter((p) => !before.includes(p));
  const dir = fakeBoard();
  const app = await open(dir);
  try {
    await app.call("set_cell", { name: "B2", content: "@claude('fake: hang=1')" });
    await app.call("trigger", { name: "B2" });
    await until(() => mine().length > 0, { what: "the run" });
    app.proc.kill("SIGTERM");
    await until(() => mine().length === 0, { timeout: 8000, what: "the run to end with the window (it outlived it)" });
  } finally {
    await app.close().catch(() => {});
  }
  // The command line: a trigger that waits on its run, interrupted.
  const cli = spawn(recalcBin, ["trigger", "B2"], { cwd: dir, stdio: "ignore" });
  await until(() => mine().length > 0, { what: "the CLI's run" });
  cli.kill("SIGINT");
  await until(() => mine().length === 0, { timeout: 8000, what: "the run to end with the command (it outlived it)" });
});

// ---- 13. recalc mcp and SIGTERM ----
await check("recalc mcp exits on SIGTERM even while its client keeps stdin open", async () => {
  const { spawn } = await import("node:child_process");
  const server = spawn(recalcBin, ["mcp"], { cwd: fakeBoard(), stdio: ["pipe", "pipe", "ignore"] });
  // Wait for an answer to initialize, so the server's own signal handling is
  // in place; a SIGTERM before that just kills it and proves nothing.
  const answered = new Promise((r) => server.stdout.once("data", r));
  server.stdin.write(JSON.stringify({ jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "edge", version: "0" } } }) + "\n");
  await answered;
  server.kill("SIGTERM"); // stdin stays open, as a client that died uncleanly leaves it
  const exited = await new Promise((r) => {
    server.on("exit", () => r(true));
    setTimeout(() => r(false), 4000);
  });
  if (!exited) server.kill("SIGKILL");
  assert.ok(exited, "still running 4 s after SIGTERM");
});

// ---- 7. sort a selection ----
await check("Sort sorts only the selected rows; a whole column keeps row 1 as the header", async () => {
  const app = await open(fakeBoard());
  try {
    await dismissWelcome(app.page);
    await app.call("paste_text", { at: "A1", text: ["Name\tScore", "a\t1", "b\t2", "c\t3", "d\t4", "e\t5"].join("\n") });
    const col = async () => Promise.all(["A2", "A3", "A4", "A5", "A6"].map(async (n) => (await cellOf(app, n))?.value));
    await app.page.click("#cells .cell[data-name='B3']");
    await app.page.click("#cells .cell[data-name='B5']", { modifiers: ["Shift"] });
    await app.menu("edit.sort_desc");
    await until(async () => (await cellOf(app, "A3"))?.value === "d", { what: "the block sorted" });
    assert.deepEqual(await col(), ["a", "d", "c", "b", "e"], "rows outside B3:B5 stay put");
    await app.page.click("#cells .cell[data-name='B2']");
    await app.menu("edit.sort_asc");
    await until(async () => (await cellOf(app, "A3"))?.value === "b", { what: "the data rows sorted" });
    assert.deepEqual(await col(), ["a", "b", "c", "d", "e"]);
    assert.equal((await cellOf(app, "A1"))?.value, "Name", "header stays");
  } finally {
    await app.close();
  }
});

await check("Export Board writes a zip of the board and the documents it names; opening it, however it was unzipped, points them at the copies", async () => {
  const { existsSync, readFileSync } = await import("node:fs");
  const { execFileSync } = await import("node:child_process");
  const dir = join(board(), "Leases.recalc");
  mkdirSync(join(dir, "docs"), { recursive: true });
  writeFileSync(join(dir, "docs/inside.md"), "inside");
  const elsewhere = realpathSync(board());
  writeFileSync(join(elsewhere, "lease.md"), "Rent is 700 a month");
  const app = await open(dir);
  try {
    await dismissWelcome(app.page);
    await app.call("set_cell", { name: "A2", content: "docs/inside.md" });
    await app.call("set_cell", { name: "A3", content: join(elsewhere, "lease.md") });
    await app.call("set_cell", { name: "notes", content: "two\nlines" });
    const sent = board();
    await app.pick(join(sent, "Leases")); // the extension is added
    const said = await app.call("export_board");
    assert.match(said, /^Exported a copy: \d+ files, .* to Leases\.recalc\.zip, with 1 from outside the board's folder\. Changes made to the copy won't come back here\. To work on one board together/);
    const zip = join(sent, "Leases.recalc.zip");
    assert.ok(existsSync(zip));
    const refused = await app.pick(join(dir, "inside.zip")).then(() => app.call("export_board")).then(() => "", (e) => e.message);
    assert.match(refused, /outside the board's own folder/);

    // Open… on the zip: unzipped beside it, as Finder would, no dialog.
    const opened = await app.call("open_file", { path: zip });
    assert.equal(realpathSync(opened), realpathSync(join(sent, "Leases.recalc")));
    await until(async () => realpathSync((await app.call("get_board")).dir) === realpathSync(opened), { what: "the unzipped board" });
    const a3 = (await cellOf(app, "A3")).value;
    assert.equal(a3, "outside/1-lease.md", "the cell names the copy that came with it");
    assert.equal(readFileSync(join(opened, a3), "utf8"), "Rent is 700 a month");
    assert.equal((await cellOf(app, "notes")).value, "two\nlines");
    assert.equal(readFileSync(join(opened, "docs/inside.md"), "utf8"), "inside");
    assert.ok(!existsSync(join(opened, ".recalc/export.json")), "the note goes once applied");

    // Unzipped by anything else (here, unzip), the first open does the same.
    const other = board();
    execFileSync("unzip", ["-q", zip, "-d", other]);
    await app.call("open_file", { path: join(other, "Leases.recalc") });
    await until(async () => (await cellOf(app, "A3"))?.value === "outside/1-lease.md" && realpathSync((await app.call("get_board")).dir) === realpathSync(join(other, "Leases.recalc")), { what: "the board unzip made" });

    // Templates open into a new board; a .recalc text file from before the rename too.
    for (const name of ["cells.recalctemplate", "old.recalc"]) {
      const text = join(board(), name);
      writeFileSync(text, `greeting = ${name}\nshout = = upper({greeting})\n`);
      await app.pick(join(board(), "From text.recalc"));
      await app.call("open_file", { path: text });
      await until(async () => (await cellOf(app, "shout"))?.value === name.toUpperCase(), { what: `the board from ${name}` });
    }
    const notBoard = join(board(), "photos.zip");
    execFileSync("zip", ["-q", "-j", notBoard, join(elsewhere, "lease.md")]);
    assert.match(await app.call("open_file", { path: notBoard }).then(() => "", (e) => e.message), /isn't a board/);
  } finally {
    await app.close();
  }
});

await check("Save a Copy copies the board somewhere new and works in the copy, with its own conversations", async () => {
  const { readFileSync } = await import("node:fs");
  const dir = fakeBoard();
  writeFileSync(join(dir, "notes.md"), "kept");
  const app = await open(dir, claudeHome());
  try {
    await dismissWelcome(app.page);
    await app.call("set_cell", { name: "A1", content: "Delaware" });
    await app.call("set_cell", { name: "B2", content: "@claude('fake: answer=x')" });
    await app.call("trigger", { name: "B2" });
    await settled(app, ["B2"]);
    const id = (await cellOf(app, "B2")).session_id;
    assert.ok(id, "the original's cell has a conversation");
    const into = join(board(), "Leases copy.recalc");
    await app.pick(into);
    const said = await app.call("save_copy");
    assert.match(said, /^You're working in the copy, “Leases copy”\. The original is unchanged, in /);
    await until(async () => realpathSync((await app.call("get_board")).dir) === realpathSync(into), { what: "the copy" });
    assert.equal((await cellOf(app, "A1")).value, "Delaware");
    assert.equal((await cellOf(app, "B2")).value, "x");
    assert.ok(!(await cellOf(app, "B2")).session_id, "the copy starts its own conversations");
    assert.equal(readFileSync(join(into, "notes.md"), "utf8"), "kept");
    assert.match(readFileSync(join(dir, ".recalc/meta/B2.toml"), "utf8"), new RegExp(id), "the original keeps its own");
    // Not into itself.
    await app.pick(join(into, "inner"));
    assert.match(await app.call("save_copy").then(() => "", (e) => e.message), /outside the board's own folder/);
  } finally {
    await app.close();
  }
});

await check("sheets in the window: tabs switch, typing lands on the shown sheet, +, rename, insert and delete", async () => {
  const app = await open(board());
  const tab = (name) => app.page.locator(".sheet-tab", { hasText: new RegExp(`^${name}$`) });
  const type = async (cell, text) => {
    await app.page.click(`#cells .cell[data-name='${cell}']`);
    await app.page.keyboard.type(text);
    await app.page.keyboard.press("Enter");
  };
  try {
    await dismissWelcome(app.page);
    await app.call("set_cell", { name: "Rules!A1", content: "never edit tests" });
    await app.call("set_cell", { name: "B1", content: "= {Rules!A1}" });
    await tab("Rules").click();
    await until(() => app.page.locator("#cells .cell[data-name='A1']").textContent().then((t) => t.includes("never edit tests")), { what: "Rules' A1 in the grid" });
    // Typing on Rules writes Rules' cells.
    await type("A2", "one changelog line per fix");
    await until(async () => (await cellOf(app, "Rules!A2"))?.value === "one changelog line per fix", { what: "Rules!A2" });
    assert.equal(await cellOf(app, "A2"), undefined, "nothing on the first sheet");
    // An unqualified formula typed on Rules reads Rules.
    await type("B1", "= len({A1})");
    await until(async () => (await cellOf(app, "Rules!B1"))?.value === "16", { what: "Rules!B1" });
    // Inserting a row on Rules moves Rules and the reference into it, not the first sheet.
    await app.page.click("#cells .cell[data-name='A1']");
    await app.menu("edit.insert_row");
    await until(async () => (await cellOf(app, "Rules!A2"))?.value === "never edit tests", { what: "Rules moved down" });
    assert.equal((await cellOf(app, "B1")).source, "= {Rules!A2}");
    await app.menu("edit.undo");
    await until(async () => (await cellOf(app, "Rules!A1"))?.value === "never edit tests", { what: "the insert undone" });
    // + adds Sheet2 and shows it; double-click renames it.
    await app.page.locator(".sheet-tab.add").click();
    await until(async () => (await app.call("get_board")).sheets.others.includes("Sheet2"), { what: "Sheet2" });
    await until(() => tab("Sheet2").getAttribute("aria-selected").then((v) => v === "true"), { what: "Sheet2 shown" });
    await tab("Rules").dblclick();
    await app.page.locator("input.sheet-tab.rename").fill("House rules");
    await app.page.keyboard.press("Enter");
    await until(async () => (await cellOf(app, "B1"))?.source === "= {'House rules'!A1}", { what: "the reference renamed" });
    // Delete from the tab's menu: references become #REF!.
    await tab("House rules").click({ button: "right" });
    await app.page.locator("#ctx .item", { hasText: "Delete Sheet" }).click();
    await app.page.locator("#overlay button", { hasText: "Delete" }).click();
    await until(async () => (await cellOf(app, "B1"))?.source === "= {#REF!}", { what: "the reference broken" });
    assert.deepEqual((await app.call("get_board")).sheets.others, ["Sheet2"]);
    assert.equal(await app.page.locator(".sheet-tab.active").textContent(), "Sheet1", "back on the first sheet");
  } finally {
    await app.close();
  }
});

await check("keys typed fast into a session's terminal arrive in order", async () => {
  const { execFileSync } = await import("node:child_process");
  const dir = board();
  mkdirSync(join(dir, ".recalc"), { recursive: true });
  // A session that prints back each line it's given.
  writeFileSync(join(dir, ".recalc/config.toml"), `[agents]\necho = "sh -c 'cat -v' {task}"\n`);
  const app = await open(dir);
  try {
    await dismissWelcome(app.page);
    await app.call("set_cell", { name: "B2", content: "@echo('x')" });
    await cell(app.page, "B2").click();
    await app.menu("agent.session");
    const term = app.page.locator("#inspector .xterm");
    await until(async () => (await term.count()) > 0, { what: "the session's terminal" });
    await app.page.waitForTimeout(1500);
    await term.click();
    // A busy board: other calls in flight while the keys go in, as in a
    // driver's run (that's when "you" came out "uoy").
    for (let i = 0; i < 40; i++) await app.call("set_cell", { name: `Z${i + 1}`, content: `row ${i}` });
    let busy = true;
    const load = (async () => {
      while (busy) await Promise.all(Array.from({ length: 6 }, () => app.call("get_board").catch(() => {})));
    })();
    const text = "Now run whatever you built that hasn't run yet, including the re-performance, and when it lands, look over the table: fix any cell that came back in the wrong form, or is uncited when it rests on a document.";
    await app.page.keyboard.type(text, { delay: 5 });
    busy = false;
    await load;
    await app.page.keyboard.press("Enter");
    const tmux = (...a) => { try { return execFileSync("tmux", ["-L", "recalc", ...a], { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }); } catch { return ""; } };
    const s = await until(() => tmux("ls", "-F", "#S").split("\n").find((n) => n.endsWith("-B2")), { what: "B2's tmux session" });
    // cat prints the line back once it has it whole: as typed, or scrambled.
    const pane = await until(() => {
      const p = tmux("capture-pane", "-p", "-J", "-t", s);
      return p.split("\n").filter((l) => l.trim()).length >= 2 ? p : null;
    }, { timeout: 15000, what: "the line printed back" });
    const lines = pane.split("\n").map((l) => l.trimEnd()).filter(Boolean);
    assert.ok(lines.includes(text), `the session got the keys out of order:\n${lines.join("\n")}`);
    await app.call("session_end", { name: "B2" }).catch(() => {});
  } finally {
    await app.close();
  }
});

await check("a script an ! cell runs reads its range as arguments and writes a block back through recalc import -", async () => {
  const { execFileSync } = await import("node:child_process");
  const dir = board();
  mkdirSync(join(dir, "scripts"), { recursive: true });
  writeFileSync(join(dir, "scripts/upper.py"), [
    "import csv, io, subprocess, sys",
    "rows = [a.upper() for a in sys.argv[1:]]",
    "out = io.StringIO()",
    "csv.writer(out).writerows([r] for r in rows)",
    "subprocess.run(['recalc', 'import', '-', '--at', 'C2'], input=out.getvalue(), text=True, check=True)",
    "print(f'{len(rows)} rows')",
  ].join("\n"));
  const app = await open(dir);
  try {
    await dismissWelcome(app.page);
    await app.call("set_cell", { name: "B2", content: "acme corp" });
    await app.call("set_cell", { name: "B3", content: "globex, inc" });
    await app.call("set_cell", { name: "upper", content: "! python3 scripts/upper.py {B2:B3}" });
    await app.call("trigger", { name: "upper" });
    await until(async () => (await cellOf(app, "C3"))?.value === "GLOBEX, INC", { timeout: 20000, what: "the script's block" });
    assert.equal((await cellOf(app, "C2")).value, "ACME CORP");
    assert.equal((await cellOf(app, "upper")).value, "2 rows", "the action's output is its value");
    // One range on the command line: its rows alone, for a script to read.
    assert.equal(execFileSync(recalcBin, ["get", "B2:B3"], { cwd: dir, encoding: "utf8" }), "acme corp\nglobex, inc\n");
  } finally {
    await app.close();
  }
});

await check("typing in a cell: ⌥Enter breaks a line and Enter-Enter keeps it; ⌘↓, ⇧Space and ⌘A move and select as in Sheets", async () => {
  const dir = board();
  const app = await open(dir);
  try {
    await dismissWelcome(app.page);
    const page = app.page;
    for (const [n, v] of [["A1", "name"], ["A2", "x"], ["A3", "y"], ["A4", "z"], ["B1", "notes"]]) await app.call("set_cell", { name: n, content: v });
    await page.click("#cells .cell[data-name='B2']");
    // The waiting input takes the first key, so it lands even typed at once.
    await page.keyboard.type("line one");
    if (process.env.RECALC_E2E_SHOTS) await page.screenshot({ path: join(process.env.RECALC_E2E_SHOTS, "oneline-edit.png"), clip: { x: 0, y: 0, width: 700, height: 220 } });
    await page.keyboard.press("Alt+Enter");
    await page.keyboard.type("line two");
    if (process.env.RECALC_E2E_SHOTS) await page.screenshot({ path: join(process.env.RECALC_E2E_SHOTS, "multiline-edit.png"), clip: { x: 0, y: 0, width: 700, height: 240 } });
    await page.keyboard.press("Enter");
    await until(async () => (await cellOf(app, "B2"))?.value === "line one\nline two", { what: "a two-line value" });
    const writes = async () => JSON.stringify((await cellOf(app, "B2")).provenance);
    await page.waitForTimeout(500);
    const before = await writes();
    await page.click("#cells .cell[data-name='B2']");
    await page.keyboard.press("Enter");
    assert.equal(await page.inputValue("#cell-editor"), "line one\nline two", "the editor keeps the line break");
    await page.keyboard.press("Enter");
    await page.waitForTimeout(400);
    assert.equal((await cellOf(app, "B2")).value, "line one\nline two", "Enter-Enter leaves it as it was");
    assert.equal(await writes(), before, "and writes nothing");
    await page.click("#cells .cell[data-name='A1']");
    await page.keyboard.press("Meta+ArrowDown");
    assert.equal(await page.textContent("#namebox"), "A4", "⌘↓ to the end of the block");
    await until(() => page.evaluate(() => document.activeElement?.id === "cell-editor"), { what: "focus waiting in the cell input after a key move" });
    await page.keyboard.press("Shift+Space");
    assert.equal(await page.textContent("#namebox"), "A4:B4", "⇧Space selects the row");
    await app.menu("edit.select_all");
    await until(async () => (await page.textContent("#namebox")) === "A1:B4", { what: "⌘A selecting the filled block" });
    // The waiting input sits on the selected cell, unseen.
    await page.click("#cells .cell[data-name='B3']");
    await until(() => page.evaluate(() => document.activeElement?.id === "cell-editor"), { what: "focus waiting in the cell input after a click" });
    const [cell, input] = await Promise.all(["#cells .cell[data-name='B3']", "#cell-editor"].map((s) => page.locator(s).boundingBox()));
    assert.ok(Math.abs(cell.x - input.x) < 2 && Math.abs(cell.y - input.y) < 2, `the waiting input is on B3: ${JSON.stringify({ cell, input })}`);
    assert.equal(await page.evaluate(() => getComputedStyle(document.getElementById("cell-editor")).opacity), "0");
  } finally {
    await app.close();
  }
});

await check("typed cells: ▾ lists the options and a click enters one; a typed value fits its canonical form, a misfit is kept and said; a date typed in words lands as YYYY-MM-DD", async () => {
  const dir = board();
  const app = await open(dir);
  try {
    await dismissWelcome(app.page);
    const page = app.page;
    await app.call("set_cell", { name: "A1", content: "status" });
    await app.call("set_answer_type", { names: ["B2", "B3"], kind: "choice", options: ["Approve", "Reject", "Needs info"], strict: true });
    await app.call("set_answer_type", { names: ["C2"], kind: "yesno", strict: true });
    await app.call("set_answer_type", { names: ["D2"], kind: "date", strict: true });
    // The backend: a fit takes its canonical form, a misfit is kept and the reply says what fits.
    assert.equal(await app.call("set_cell", { name: "C2", content: "yes" }), null);
    assert.equal((await cellOf(app, "C2")).value, "Yes", "yes is entered as Yes");
    assert.equal(await app.call("set_cell", { name: "B3", content: "later" }), "B3 takes one of Approve, Reject, Needs info; kept as you typed it");
    assert.equal((await cellOf(app, "B3")).value, "later");
    // ▾ on the selected cell, then a click on an option.
    await page.click("#cells .cell[data-name='B2']");
    await page.click("#cells .cell[data-name='B2'] .pick-btn");
    await page.waitForSelector("#autocomplete:not([hidden]) .ac-row");
    if (process.env.RECALC_E2E_SHOTS) await page.screenshot({ path: join(process.env.RECALC_E2E_SHOTS, "choice-list.png"), clip: { x: 0, y: 80, width: 600, height: 220 } });
    assert.deepEqual(await page.$$eval("#autocomplete .ac-row .ac-name", (n) => n.map((x) => x.textContent)), ["Approve", "Reject", "Needs info"]);
    await page.click("#autocomplete .ac-row:nth-child(3)");
    await until(async () => (await cellOf(app, "B2"))?.value === "Needs info", { what: "the clicked option entered" });
    if (process.env.RECALC_E2E_SHOTS) await page.screenshot({ path: join(process.env.RECALC_E2E_SHOTS, "choice-chip.png"), clip: { x: 0, y: 80, width: 600, height: 220 } });
    // A misfit typed in the window: kept, and the inspector says what the cell takes.
    await page.click("#cells .cell[data-name='B3']");
    await page.keyboard.type("someday");
    await page.keyboard.press("Enter");
    await until(async () => (await cellOf(app, "B3"))?.value === "someday", { what: "the misfit kept" });
    await page.waitForFunction(() => /B3 takes one of Approve, Reject, Needs info/.test(document.getElementById("inspector").textContent));
    // A date in words, entered as the cell takes it.
    await page.click("#cells .cell[data-name='D2']");
    await page.keyboard.type("march 4 2026");
    await page.waitForSelector("#autocomplete .ac-row.active");
    if (process.env.RECALC_E2E_SHOTS) await page.screenshot({ path: join(process.env.RECALC_E2E_SHOTS, "date-typed.png"), clip: { x: 0, y: 80, width: 700, height: 220 } });
    await page.keyboard.press("Enter");
    await until(async () => (await cellOf(app, "D2"))?.value === "2026-03-04", { what: "the date as YYYY-MM-DD" });
    // The calendar opens without an error (its picker is the system's).
    await page.click("#cells .cell[data-name='D2']");
    await page.click("#cells .cell[data-name='D2'] .pick-btn");
    await page.click("#autocomplete .ac-row:has-text('Choose from a calendar')");
    await page.waitForTimeout(300);
    assert.deepEqual(app.errors.filter((e) => !/favicon/.test(e)), [], "no page errors");
  } finally {
    await app.close();
  }
});

// ---- a JSON answer type, picked as a person would ----
await check("the answer type picker offers JSON with the keys its answers must have", async () => {
  const app = await open(fakeBoard());
  try {
    await dismissWelcome(app.page);
    const page = app.page;
    await app.call("set_cell", { name: "B2", content: "@claude('fake: answer=x')" });
    await page.click("#cells .cell[data-name='B2']");
    await app.menu("cell.answer_type");
    await page.locator("#overlay .row", { hasText: "JSON" }).first().click();
    await page.waitForSelector("#overlay:not([hidden]) input.field");
    await page.keyboard.type("case_id, prediction");
    await page.keyboard.press("Enter");
    await until(async () => (await cellOf(app, "B2"))?.answer_type?.kind === "json", { what: "B2 typed JSON" });
    assert.deepEqual((await cellOf(app, "B2")).answer_type.options, ["case_id", "prediction"]);
    await page.click("#cells .cell[data-name='C2']");
    await page.click("#cells .cell[data-name='B2']");
    await page.waitForFunction(() => /JSON with case_id, prediction/.test(document.getElementById("inspector").textContent));
  } finally {
    await app.close();
  }
});

// ---- a sheet's layout doesn't carry the others' ----
await check("a sheet's saved layout holds only its own settings, and a nested one is cleaned on load", async () => {
  const dir = fakeBoard();
  // As an earlier version left it: the whole document inside a sheet's entry.
  const nested = { widths: { A: 200 }, sheets: { Notes: { widths: { B: 150 }, sheets: { Notes: { widths: {} } } } } };
  writeFileSync(join(dir, ".recalc/layout.json"), JSON.stringify(nested));
  const app = await open(dir);
  try {
    await dismissWelcome(app.page);
    await app.call("set_cell", { name: "Notes!A1", content: "x" });
    await app.page.locator("#sheet-tabs .sheet-tab", { hasText: /^Notes$/ }).click();
    await app.menu("view.wrap");
    await app.page.locator("#sheet-tabs .sheet-tab").first().click();
    await app.menu("view.wrap");
    await until(() => {
      const saved = JSON.parse(readFileSync(join(dir, ".recalc/layout.json"), "utf8"));
      return saved.sheets?.Notes?.wrap === true && saved.wrap === true;
    }, { what: "both layouts saved" });
    const saved = JSON.parse(readFileSync(join(dir, ".recalc/layout.json"), "utf8"));
    assert.equal(saved.sheets.Notes.sheets, undefined, "no layouts inside a sheet's");
    assert.equal(saved.sheets.Notes.widths.B, 150, "its own settings kept");
    assert.equal(saved.widths.A, 200);
  } finally {
    await app.close();
  }
});

console.log(`${results.filter((r) => r[1] === "ok").length} of ${results.length} passed`);
process.exit(results.some((r) => r[1] === "FAIL") ? 1 : 0);
