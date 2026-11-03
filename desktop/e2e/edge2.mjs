// More edge-case checks: a stand-in driver, two windows on one board,
// permission asks at scale, keyboard review, edits over computed cells,
// snoozing, the CLI gate, odd files as rows, the Codex path. Free (stand-ins).
// `cargo build -p recalc-desktop`, then `node edge2.mjs [filter]`.
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { writeFileSync, mkdirSync, readFileSync, existsSync, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { board, open, cell, until, recalc } from "./lib.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const fake = join(here, "fake-agent.py");
const fakeDriver = join(here, "fake-driver.py");
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
/** A one-page PDF of `text`, with the xref table parsers require. */
function minimalPdf(text) {
  const stream = `BT /F1 18 Tf 40 700 Td (${text}) Tj ET`;
  const objs = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>",
    `<< /Length ${stream.length} >>\nstream\n${stream}\nendstream`,
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
  ];
  let out = "%PDF-1.4\n";
  const offs = [];
  objs.forEach((o, i) => {
    offs.push(Buffer.byteLength(out));
    out += `${i + 1} 0 obj\n${o}\nendobj\n`;
  });
  const xref = Buffer.byteLength(out);
  out += `xref\n0 ${objs.length + 1}\n0000000000 65535 f \n`;
  for (const o of offs) out += `${String(o).padStart(10, "0")} 00000 n \n`;
  out += `trailer\n<< /Size ${objs.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return out;
}
async function grid(app, n, { sleep = 0.5, extra = "" } = {}) {
  mkdirSync(join(app.dir, "docs"), { recursive: true });
  const names = [];
  await app.call("set_cell", { name: "A1", content: "Document" });
  await app.call("set_cell", { name: "B1", content: "Answer" });
  for (let i = 1; i <= n; i++) {
    writeFileSync(join(app.dir, `docs/d${i}.md`), `Document ${i}. The passage of d${i} is here.\n`);
    await app.call("set_cell", { name: `A${i + 1}`, content: `docs/d${i}.md` });
    await app.call("set_cell", { name: `B${i + 1}`, content: `@claude('fake: answer=ans-d${i}; sleep=${sleep}${extra};; Document: {$A${i + 1}}')` });
    names.push(`B${i + 1}`);
  }
  return names;
}

// ---- 1. the driver's seat ----
await check("a board saved with recalc save shows in the template gallery and loads into another board", async () => {
  const src = board();
  execFileSync(recalc, ["set", "greeting", "hello"], { cwd: src });
  execFileSync(recalc, ["set", "shout", "= upper({greeting})"], { cwd: src });
  const dir = board();
  mkdirSync(join(dir, "examples"), { recursive: true });
  execFileSync(recalc, ["save", join(dir, "examples/mine.recalctemplate")], { cwd: src });
  const app = await open(dir);
  try {
    const list = await app.call("templates");
    assert.ok(list.some((t) => t.name === "examples/mine.recalctemplate"), JSON.stringify(list.map((t) => t.name)));
    await app.call("load_template", { name: "examples/mine.recalctemplate" });
    await until(async () => (await app.call("get_board")).cells.find((c) => c.name === "shout")?.value === "HELLO", { what: "the saved board's formula" });
    const refused = await app.call("load_template", { name: "examples/../../etc/x.recalc" }).then(() => "", (e) => e.message);
    assert.match(refused, /no such template/);
  } finally {
    await app.close();
  }
});

await check("with Claude Code missing, a session and the driver say to install it instead of opening and closing a terminal", async () => {
  const dir = board();
  mkdirSync(join(dir, ".recalc"), { recursive: true });
  writeFileSync(join(dir, ".recalc/config.toml"), '[paths]\nclaude = "/nonexistent/bin/claude"\n');
  const app = await open(dir);
  try {
    await app.page.keyboard.press("Escape").catch(() => {});
    await app.call("set_cell", { name: "B2", content: "@claude('hello')" });
    const refused = await app.call("session_open", { name: "B2", cols: 80, rows: 24 }).then(() => "", (e) => e.message);
    assert.match(refused, /Claude Code isn't installed/, refused);
    await app.page.click("#tab-driver");
    await app.page.locator("#tray button", { hasText: "Start" }).click();
    await until(async () => /Claude Code isn't installed/.test((await app.page.textContent("#tray")) ?? ""), { timeout: 10000, what: "the driver's reason" });
    assert.deepEqual(app.errors, []);
  } finally {
    await app.close();
  }
});

await check("a stand-in driver takes the seat, lays out the board as `driver`, cannot run its own action, and the seat ends cleanly", async () => {
  const dir = board();
  mkdirSync(join(dir, ".recalc"), { recursive: true });
  writeFileSync(join(dir, ".recalc/config.toml"), `driver = "fakedrv"\n[agents]\nfakedrv = "python3 ${fakeDriver} --mcp-config {mcp}"\n`);
  const app = await open(dir);
  try {
    await dismissWelcome(app.page);
    await app.call("driver_open", { cols: 100, rows: 30 });
    await until(async () => (await cellOf(app, "driver_report")) != null, { timeout: 20000, what: "the driver to lay out the board" });
    const b = await app.call("get_board");
    const plan = b.cells.find((c) => c.name === "plan_1");
    assert.equal(plan.value, "Lay out the table");
    assert.equal(b.cells.find((c) => c.name === "plan_1_status").value, "done", "the named-form update landed");
    assert.equal(b.cells.find((c) => c.name === "plan_2").value, "Set up the commit");
    assert.equal(String(plan.provenance?.by), "agent:driver", `its writes read driver: ${JSON.stringify(plan.provenance)}`);
    // The driver filled a formula down and then inserted a row above the table.
    const at = (n) => b.cells.find((c) => c.name === n);
    assert.equal(at("B3").value, "9", "the table's formula ran (len of docs/a.md) and moved down a row");
    assert.equal(at("C3").value, "docs/a.md/Question", "filled with anchors, then moved");
    assert.equal(at("C4").value, "docs/b.md/Question");
    assert.ok(!at("A2")?.value, "the inserted row is empty");
    const report = JSON.parse(b.cells.find((c) => c.name === "driver_report").value);
    const named = report.filter((r) => r.tool === "set_range" && r.args.cells);
    assert.ok(named.length === 2 && named.every((r) => !r.isError), `the plan went in by name: ${JSON.stringify(named).slice(0, 200)}`);
    const read = report.find((r) => r.tool === "get_range" && r.args.names);
    assert.deepEqual(JSON.parse(read.text), { plan_1: "Lay out the table", plan_1_status: "done", plan_2_status: "todo" });
    const trig = report.find((r) => r.tool === "trigger_cell");
    assert.ok(trig.isError && /person|human|first/i.test(trig.text), `the driver's own action refused until a person runs it: ${trig.text.slice(0, 160)}`);
    assert.ok(!existsSync(join(dir, "committed.txt")), "and it did not run");
    const status = await app.call("driver_status");
    assert.equal(status.status, "running", `seat is live: ${JSON.stringify(status)}`);
    assert.equal(status.agent, "fakedrv");
    // Once a person has run the action, the driver may re-trigger it.
    await app.call("trigger", { name: "commit_cmd" });
    await settled(app, ["commit_cmd"]);
    assert.ok(existsSync(join(dir, "committed.txt")), "the person's run did it");
    await app.page.click("#tab-driver");
    await app.page.locator("#tray button", { hasText: /^Show$/ }).first().click();
    await until(async () => ((await app.page.textContent("#tray")) ?? "").includes("fake driver"), { timeout: 15000, what: "the seat's terminal to show the driver" });
    await app.shot("edge2-driver-seat");
    await app.call("driver_end");
    await app.page.waitForTimeout(800);
    const after = await app.call("driver_status");
    assert.equal(after.status, "none", `seat ended: ${JSON.stringify(after)}`);
    assert.deepEqual(app.errors, []);
  } finally {
    await app.close();
  }
});

// ---- 2. identical permission asks at scale ----
await check("six identical permission asks land as one Needs You group, and Allow all answers every run", async () => {
  const app = await open(fakeBoard("max_agents = 6"));
  try {
    await dismissWelcome(app.page);
    const names = await grid(app, 6, { extra: "; perm=Bash|mkdir probe-dir; answer=allowed" });
    await app.call("run_selection", { names });
    await app.page.click("#tab-decisions");
    await until(async () => ((await app.page.textContent("#tray")) ?? "").includes("Allow all 6 once"), { timeout: 20000, what: "the grouped row" });
    const count = (await app.page.textContent("#needs-count"))?.trim();
    assert.equal(count, "6", `the tab counts every ask: ${count}`);
    await app.shot("edge2-grouped-permissions");
    await app.page.locator("#tray button", { hasText: "Allow all 6 once" }).first().click();
    await settled(app, names, 30000);
    for (const n of names) assert.equal((await cellOf(app, n)).value, "allowed", n);
    await until(async () => !((await app.page.textContent("#tray")) ?? "").includes("Allow all"), { timeout: 5000, what: "the group to leave the list" });
  } finally {
    await app.close();
  }
});

// ---- 3. two windows on one board ----
await check("a second window on the same board mirrors a run, and takes over when the first one dies", async () => {
  const dir = fakeBoard();
  const a = await open(dir);
  let b;
  try {
    await dismissWelcome(a.page);
    b = await open(dir);
    await dismissWelcome(b.page);
    await a.call("set_cell", { name: "B2", content: "@claude('fake: answer=from-a; sleep=8')" });
    await a.call("trigger", { name: "B2" });
    await until(async () => (await cellOf(b, "B2"))?.state === "computing", { timeout: 10000, what: "the second window to see the run" });
    await b.call("set_cell", { name: "C2", content: "= 1 + 1" });
    await until(async () => (await cellOf(a, "C2"))?.value === "2", { timeout: 10000, what: "the first window to see the second's formula" });
    a.proc.kill("SIGKILL");
    await a.page.context().browser().close().catch(() => {});
    // The run's owner died: the survivor must reset the cell and compute again.
    await until(async () => (await cellOf(b, "B2"))?.state !== "computing", { timeout: 20000, what: "the orphaned run to be reset" });
    const c = await cellOf(b, "B2");
    assert.ok(c.state === "stale" || st(c).startsWith("error"), `reset, not running: ${st(c)}`);
    await b.call("set_cell", { name: "D2", content: "$ echo alive" });
    await until(async () => (await cellOf(b, "D2"))?.value === "alive", { timeout: 15000, what: "the survivor to run commands" });
    await b.call("trigger", { name: "B2" });
    await settled(b, ["B2"], 30000);
    assert.equal((await cellOf(b, "B2")).value, "from-a");
  } finally {
    await b?.close();
    try {
      execFileSync("pkill", ["-f", "fake-agent.py"], { stdio: "ignore" });
    } catch {}
  }
});

// ---- 4. the keyboard review pass ----
await check("Review Answers walks a column by keyboard: V verifies, F flags with a note, S skips, Esc ends with a tally, and the filter shows what's left", async () => {
  const app = await open(fakeBoard("max_agents = 6"));
  try {
    await dismissWelcome(app.page);
    const names = await grid(app, 6, { sleep: 0.2 });
    await app.call("run_selection", { names });
    await settled(app, names);
    await cell(app.page, "B2").click();
    await app.menu("cell.review");
    await until(async () => /Reviewing 1 of 6/.test((await app.page.textContent("#inspector")) ?? ""), { timeout: 5000, what: "the review to start" });
    await app.shot("edge2-review-start");
    for (const key of ["v", "v", "f", "s", "v"]) {
      await app.page.keyboard.press(key);
      if (key === "f") {
        await app.page.waitForSelector("#overlay:not([hidden]) input", { timeout: 5000 });
        await app.page.keyboard.type("needs a second look");
        await app.page.keyboard.press("Enter");
        await app.page.waitForSelector("#overlay", { state: "hidden", timeout: 5000 });
      }
      await app.page.waitForTimeout(350);
    }
    await until(async () => /Reviewing 6 of 6/.test((await app.page.textContent("#inspector")) ?? ""), { timeout: 5000, what: "the last cell" });
    await app.page.keyboard.press("Escape");
    await app.page.waitForTimeout(400);
    const b = await app.call("get_board");
    const marks = names.map((n) => b.cells.find((c) => c.name === n).mark?.kind ?? "-");
    assert.deepEqual(marks, ["verified", "verified", "flagged", "-", "verified", "-"], `marks in order: ${marks}`);
    assert.equal(b.cells.find((c) => c.name === "B4").mark?.note, "needs a second look");
    const inspector = (await app.page.textContent("#inspector")) ?? "";
    assert.ok(/3 verified/.test(inspector) && /1 flagged/.test(inspector), `the tally: ${inspector.slice(0, 200)}`);
    await app.menu("view.filter_unverified");
    await app.page.waitForTimeout(400);
    const visible = await app.page.evaluate(() => [...document.querySelectorAll("#cells .cell[data-name^='A'], #frozen-cols .cell[data-name^='A']")].filter((e) => e.offsetParent !== null).map((e) => e.dataset.name));
    assert.ok(visible.includes("A5") && visible.includes("A7") && !visible.includes("A2"), `only unverified rows show: ${visible}`);
    await app.shot("edge2-review-filter");
    await app.menu("view.filter_all");
  } finally {
    await app.close();
  }
});

// ---- 5. editing over a computed cell ----
await check("typing over an agent's answer asks suggest / pin / override, and each does what it says", async () => {
  const app = await open(fakeBoard());
  try {
    await dismissWelcome(app.page);
    await app.call("set_cell", { name: "B2", content: "@claude('fake: answer=first')" });
    await app.call("set_cell", { name: "B3", content: "@claude('fake: answer=third')" });
    await app.call("run_selection", { names: ["B2", "B3"] });
    await settled(app, ["B2", "B3"]);
    await app.call("edit_computed", { name: "B2", typed: "mine" });
    let c = await cellOf(app, "B2");
    assert.ok(c.decision, "a decision is posed");
    let labels = c.decision.options.map((o) => o.value);
    assert.deepEqual(labels, ["pin", "override", "cancel"], `a cell the person defined has nobody to suggest to: ${labels}`);
    await app.call("resolve_decision", { name: "B2", index: 1 });
    await until(async () => (await cellOf(app, "B2")).value === "mine", { what: "override" });
    c = await cellOf(app, "B2");
    assert.equal(c.kind, "agent_call", "override keeps the source");
    await app.call("edit_computed", { name: "B3", typed: "pinned" });
    await app.call("resolve_decision", { name: "B3", index: 0 });
    await until(async () => (await cellOf(app, "B3")).value === "pinned", { what: "pin" });
    c = await cellOf(app, "B3");
    assert.equal(c.kind, "literal", "pin makes it a value");
    assert.equal(c.source, null);
    await app.menu("edit.undo");
    await app.page.waitForTimeout(500);
    const after = await cellOf(app, "B3");
    assert.equal(after?.kind, "agent_call", `undo after pin puts the agent cell back, not what came before it: ${after?.kind} ${JSON.stringify(after?.value)}`);
    assert.ok(after.source.startsWith("@claude"), `the source is back: ${after.source}`);
    console.log(`     undo after pin: ${after.kind}, ${st(after)}, value ${JSON.stringify(after.value)} (the source is back; the value is out of date until it runs)`);
    // Editing a cell an agent wrote offers to suggest the edit instead.
    await app.call("set_cell", { name: "C2", content: `@claude('fake: set=B4|@claude("fake: answer=fourth")')` });
    await app.call("trigger", { name: "C2" });
    await settled(app, ["C2"]);
    await app.call("trigger", { name: "B4" });
    await settled(app, ["B4"]);
    assert.equal((await cellOf(app, "B4")).value, "fourth");
    await app.call("edit_computed", { name: "B4", typed: "no, fifth" });
    c = await cellOf(app, "B4");
    labels = c.decision.options.map((o) => o.value);
    assert.equal(labels[0], "suggest", `an agent-owned cell offers suggest first: ${labels}`);
    await app.call("resolve_decision", { name: "B4", index: 0 });
    await app.page.waitForTimeout(300);
    c = await cellOf(app, "B4");
    assert.equal(c.value, "fourth", "suggesting leaves the value");
    assert.ok(c.pending_suggestions.some((s) => s.value === "no, fifth"), `the suggestion is recorded for the agent: ${JSON.stringify(c.pending_suggestions)}`);
  } finally {
    await app.close();
  }
});

// ---- 6. suggestions: adopt and dismiss ----
await check("a suggestion on a verified cell can be adopted from the Inspector or dismissed", async () => {
  const app = await open(fakeBoard());
  try {
    await dismissWelcome(app.page);
    await app.call("set_cell", { name: "B2", content: "@claude('fake: answer=first')" });
    await app.call("trigger", { name: "B2" });
    await settled(app, ["B2"]);
    await app.call("set_mark", { names: ["B2"], kind: "verified" });
    await app.call("set_cell", { name: "C2", content: "@claude('fake: set=B2|second; set=B2|third')" });
    await app.call("trigger", { name: "C2" });
    await settled(app, ["C2"]);
    let c = await cellOf(app, "B2");
    assert.equal(c.value, "first");
    assert.ok(c.pending_suggestions.length >= 1, `suggestions kept: ${JSON.stringify(c.pending_suggestions)}`);
    await app.call("dismiss_suggestion", { name: "B2", value: c.pending_suggestions[0].value });
    c = await cellOf(app, "B2");
    const left = c.pending_suggestions.length;
    await cell(app.page, "B2").click();
    await app.page.waitForTimeout(300);
    const adopt = app.page.locator("#inspector button", { hasText: "Adopt" });
    if (left > 0) {
      assert.ok((await adopt.count()) >= 1, "the Inspector offers to adopt what's left");
      await adopt.first().click();
      await until(async () => (await cellOf(app, "B2")).value !== "first", { what: "adopt" });
      c = await cellOf(app, "B2");
      assert.equal(c.mark?.kind, "verified", "adopting keeps the person's mark; it was their choice");
      assert.equal(c.pending_suggestions.length, 0);
    } else {
      assert.equal(await adopt.count(), 0, "nothing left to adopt");
    }
  } finally {
    await app.close();
  }
});

// ---- 7. Later ----
await check("a flag can be put aside with Later and comes back; a question can't, since an agent is waiting", async () => {
  const app = await open(fakeBoard());
  try {
    await dismissWelcome(app.page);
    await app.call("set_cell", { name: "A1", content: "look at me" });
    await app.call("set_mark", { names: ["A1"], kind: "flagged", note: "check the total" });
    await app.call("set_cell", { name: "B2", content: "@claude('fake: ask=tier|Which tier?|GREEN|RED')" });
    await app.call("trigger", { name: "B2" });
    await app.page.click("#tab-decisions");
    await until(async () => ((await app.page.textContent("#tray")) ?? "").includes("Which tier?"), { what: "the question" });
    const later = app.page.locator("#tray .zz", { hasText: "Later" });
    assert.equal(await later.count(), 1, "only the flag offers Later; the question doesn't");
    await later.first().click({ force: true });
    await app.page.locator(".snooze-menu .item", { hasText: "for an hour" }).first().click();
    await app.page.waitForTimeout(400);
    await app.shot("edge2-snoozed");
    const tray = (await app.page.textContent("#tray")) ?? "";
    assert.ok(/Later|aside|snoozed/i.test(tray), `the shelf says what was put aside: ${tray.slice(0, 200)}`);
    const main = tray.split(/Later|Put aside/i)[0];
    assert.ok(!/check the total/.test(main), "the flag left the main list");
    await app.call("resolve_decision", { name: "tier", index: 1 });
    await settled(app, ["B2"]);
    assert.equal((await cellOf(app, "B2")).value, "RED");
  } finally {
    await app.close();
  }
});

// ---- 8. the CLI gate ----
await check("recalc check applies a board and exits by the gate cell's value and state", async () => {
  const dir = mkdtempSync(join(tmpdir(), "recalc-check-"));
  const tpl = join(dir, "gate.recalc");
  writeFileSync(tpl, "status = $ test -f ok && echo true || echo false\nbroken = = {nope} + 1\n");
  let r = spawnSync(recalc, ["check", tpl, "--cell", "status", "--timeout", "5000"], { cwd: dir, encoding: "utf8" });
  assert.notEqual(r.status, 0, `no ok file: ${r.stdout} ${r.stderr}`);
  writeFileSync(join(dir, "ok"), "");
  r = spawnSync(recalc, ["check", tpl, "--cell", "status", "--timeout", "5000"], { cwd: dir, encoding: "utf8" });
  assert.equal(r.status, 0, `with the file: ${r.stdout} ${r.stderr}`);
  r = spawnSync(recalc, ["check", tpl, "--cell", "broken", "--timeout", "5000"], { cwd: dir, encoding: "utf8" });
  assert.notEqual(r.status, 0, "an errored gate is refused");
  assert.ok(/broken/.test(r.stdout + r.stderr), `the log shows why: ${(r.stdout + r.stderr).slice(0, 200)}`);
});

// ---- 9. odd files as rows ----
await check("a folder with a PDF, an empty file, a binary, a dotfile and a subfolder imports the readable rows, and Source reads the PDF", async () => {
  const app = await open(fakeBoard());
  try {
    await dismissWelcome(app.page);
    const docs = join(app.dir, "docs");
    mkdirSync(join(docs, "nested"), { recursive: true });
    writeFileSync(join(docs, "a.md"), "Rent is 500.\n");
    writeFileSync(join(docs, "empty.txt"), "");
    writeFileSync(join(docs, "pic.png"), Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0]));
    writeFileSync(join(docs, ".hidden.md"), "secret\n");
    writeFileSync(join(docs, "nested/deep.md"), "deep\n");
    writeFileSync(join(docs, "lease.pdf"), minimalPdf("Rent is 700 a month"));
    await app.pick(docs);
    const n = await app.call("import_files", { at: "A2" });
    const b = await app.call("get_board");
    const rows = b.cells.filter((c) => /^A\d+$/.test(c.name) && c.name !== "A1").map((c) => c.value).sort();
    console.log(`     imported ${n}: ${rows.join(", ")}`);
    assert.ok(!rows.some((r) => r.includes(".hidden")), "dotfiles are skipped");
    assert.ok(rows.includes("docs/a.md") && rows.includes("docs/lease.pdf"), "the readable files are rows");
    const src = await app.call("read_source", { file: "docs/lease.pdf" }).catch((e) => ({ error: e.message }));
    console.log(`     pdf source: ${JSON.stringify(src).slice(0, 160)}`);
    assert.ok(JSON.stringify(src).includes("Rent is 700"), `the PDF's text is read: ${JSON.stringify(src).slice(0, 200)}`);
    const png = await app.call("read_source", { file: "docs/pic.png" }).catch((e) => ({ error: e.message }));
    console.log(`     png source: ${JSON.stringify(png).slice(0, 120)}`);
    const empty = await app.call("read_source", { file: "docs/empty.txt" }).catch((e) => ({ error: e.message }));
    console.log(`     empty source: ${JSON.stringify(empty).slice(0, 120)}`);
    assert.deepEqual(app.errors, []);
  } finally {
    await app.close();
  }
});

// ---- 10. the Codex path with a stand-in ----
await check("a stand-in codex speaking exec --json lands its answer, cost and conversation id", async () => {
  const dir = board();
  mkdirSync(join(dir, ".recalc"), { recursive: true });
  const fakeCodex = join(dir, "codex");
  const fixture = join(here, "../../tests/fixtures/agents/codex-0.154.0/ok.jsonl");
  writeFileSync(fakeCodex, `#!/bin/sh\ncat >/dev/null\ncat ${fixture}\n`);
  execFileSync("chmod", ["755", fakeCodex]);
  writeFileSync(join(dir, ".recalc/config.toml"), `[paths]\ncodex = "${fakeCodex}"\n`);
  const app = await open(dir);
  try {
    await dismissWelcome(app.page);
    await app.call("set_cell", { name: "B2", content: "@codex('say ok')" });
    await app.call("trigger", { name: "B2" });
    await settled(app, ["B2"]);
    const c = await cellOf(app, "B2");
    console.log(`     codex value=${JSON.stringify(c.value).slice(0, 60)} cost=${c.usage?.cost_usd} session=${c.session_id?.slice(0, 8)}`);
    assert.equal(c.state, "ok", st(c));
    assert.ok(c.value.length > 0);
    assert.ok(c.usage, "usage recorded");
    assert.ok(c.session_id, "the conversation id is kept");
  } finally {
    await app.close();
  }
});

// ---- 11. undo for structural edits, and paste values ----
await check("deleting a row and sorting can be undone with references and marks restored, and Paste Values lands text, not formulas", async () => {
  const app = await open(fakeBoard());
  try {
    await dismissWelcome(app.page);
    for (const [n, v] of [["A1", "Doc"], ["B1", "Score"], ["A2", "c.md"], ["B2", "30"], ["A3", "a.md"], ["B3", "10"], ["A4", "b.md"], ["B4", "20"]]) await app.call("set_cell", { name: n, content: v });
    await app.call("set_cell", { name: "total", content: "= sum({B2:B4})" });
    await app.call("set_cell", { name: "first", content: "= {B3}" });
    await app.call("set_mark", { names: ["B3"], kind: "flagged", note: "check" });
    await until(async () => (await cellOf(app, "total"))?.value === "60", { what: "total" });
    // Delete row 3 (a.md, 10, flagged).
    await app.call("restructure", { axis: "rows", at: 3, n: 1, insert: false });
    await until(async () => (await cellOf(app, "total"))?.value === "50", { what: "total after the delete" });
    await app.menu("edit.undo");
    await until(async () => (await cellOf(app, "total"))?.value === "60", { what: "total after undo" });
    let b3 = await cellOf(app, "B3");
    assert.equal(b3.value, "10");
    assert.equal(b3.mark?.kind, "flagged", "the mark came back with the row");
    assert.equal((await cellOf(app, "first")).source, "= {B3}", "a reference into the deleted row is restored");
    await app.menu("edit.redo");
    await until(async () => (await cellOf(app, "total"))?.value === "50", { what: "total after redo" });
    await app.menu("edit.undo");
    await until(async () => (await cellOf(app, "total"))?.value === "60", { what: "total after a second undo" });
    await app.call("sort_rows", { col: 2, first: 2, last: 4, descending: false });
    await until(async () => (await cellOf(app, "A2"))?.value === "a.md", { what: "the sort" });
    await app.menu("edit.undo");
    await until(async () => (await cellOf(app, "A2"))?.value === "c.md", { what: "the sort undone" });
    assert.equal((await cellOf(app, "first")).source, "= {B3}", "the reference followed its row back");
    assert.equal((await cellOf(app, "first")).value, "10");
    // Paste values keeps formulas and commands as text.
    await app.call("paste_values", { at: "D2", block: [["60", "= 1+1", "$ echo hi"]] });
    const d2 = await cellOf(app, "D2");
    const e2 = await cellOf(app, "E2");
    const f2 = await cellOf(app, "F2");
    assert.equal(d2.kind, "literal");
    assert.equal(e2.kind, "literal", "= 1+1 lands as text");
    assert.equal(e2.value, "= 1+1");
    assert.equal(f2.kind, "literal", "a command lands as text");
    await app.menu("edit.undo");
    await until(async () => (await cellOf(app, "D2")) == null || (await cellOf(app, "D2")).value === "", { what: "paste values undone" });
    assert.deepEqual(app.errors, []);
  } finally {
    await app.close();
  }
});

console.log(`${results.filter((r) => r[1] === "ok").length} of ${results.length} passed`);
process.exit(results.some((r) => r[1] === "FAIL") ? 1 : 0);
