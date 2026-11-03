// End-to-end checks: the app driven through its UI in WebKit via bridge mode.
// Free: agents are recorded Claude sessions played back as `claude`.
// `cargo build -p recalc-desktop`, then `npm test`; screenshots go to shots/.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { appendFileSync, writeFileSync, mkdirSync, chmodSync, existsSync, cpSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { board, open, cell, until, recalc } from "./lib.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const results = [];
const only = process.argv[2];
async function check(name, f) {
  if (only && !name.includes(only)) return;
  const started = Date.now();
  try {
    await f();
    results.push([name, "ok", Date.now() - started]);
    console.log(`ok   ${name}`);
  } catch (e) {
    results.push([name, "FAIL", Date.now() - started]);
    console.log(`FAIL ${name}\n     ${String(e.message ?? e).split("\n").join("\n     ")}`);
  }
}
const cellOf = async (app, n) => (await app.call("get_board")).cells.find((c) => c.name === n);
// The welcome opens a moment after load, so wait for it rather than check once.
const dismissWelcome = async (page) => {
  const shown = await page.waitForSelector("#overlay:not([hidden])", { timeout: 5000 }).catch(() => null);
  if (shown) {
    await page.keyboard.press("Escape");
    await page.waitForSelector("#overlay", { state: "hidden", timeout: 5000 });
  }
};

await check("typing values, a formula, and fast entry land in the right cells", async () => {
  const app = await open(board());
  try {
    await dismissWelcome(app.page);
    await cell(app.page, "A1").click();
    // No key delay, as a scanner or text expander types.
    for (const v of ["12", "30", "= sum({A1:A2})"]) {
      await app.page.keyboard.type(v);
      await app.page.keyboard.press("Enter");
    }
    await until(async () => (await cellOf(app, "A3"))?.value === "42", { what: "A3 = 42" });
    assert.equal((await cellOf(app, "A1")).value, "12");
    assert.equal(await app.page.textContent("#namebox"), "A4", "Enter moved down each time");
    assert.ok(await app.page.isHidden("#cell-editor:not(.waiting)"), "the editor closed");
  } finally {
    await app.close();
  }
});

await check("a double click edits the cell in place: its text, the caret at the end, and what the keys do", async () => {
  const app = await open(board());
  try {
    await dismissWelcome(app.page);
    await app.call("set_cell", { name: "B2", content: "Acme Corp" });
    await cell(app.page, "B2").dblclick();
    const ed = app.page.locator("#cell-editor");
    await until(() => ed.evaluate((e) => !e.hidden && !e.classList.contains("waiting")), { what: "the editor open", timeout: 3000 });
    assert.equal(await ed.inputValue(), "Acme Corp", "it edits the text, not a blank");
    assert.deepEqual(await ed.evaluate((e) => [e.selectionStart, e.selectionEnd]), [9, 9], "the caret at the end");
    assert.equal(await app.page.evaluate(() => document.activeElement?.id), "cell-editor");
    assert.match(await app.page.locator("#status-editing").textContent(), /Editing B2 · Enter saves · Esc cancels/);
    assert.ok(await app.page.locator("#fill-handle").isHidden(), "no fill handle while editing");
    assert.equal(await ed.evaluate((e) => e.classList.contains("code")), false, "a value keeps the cell's font");
    await app.page.keyboard.type(" Inc");
    await app.page.keyboard.press("Enter");
    await until(async () => (await cellOf(app, "B2"))?.value === "Acme Corp Inc", { what: "the edit saved" });
    assert.ok(await app.page.locator("#status-editing").isHidden(), "the hint goes with the edit");
    await app.call("set_cell", { name: "B3", content: "= 2 + 2" });
    await cell(app.page, "B3").dblclick();
    await until(async () => (await ed.inputValue()) === "= 2 + 2", { what: "the formula's source" });
    assert.equal(await ed.evaluate((e) => e.classList.contains("code")), true, "a formula in the code font");
    await app.page.keyboard.press("Escape");
    assert.equal((await cellOf(app, "B3")).value, "4");
  } finally {
    await app.close();
  }
});

await check("undo takes back one edit and the formula follows", async () => {
  const app = await open(board());
  try {
    await dismissWelcome(app.page);
    await cell(app.page, "A1").click();
    for (const v of ["12", "30", "= sum({A1:A2})"]) {
      await app.page.keyboard.type(v, { delay: 20 });
      await app.page.keyboard.press("Enter");
    }
    await cell(app.page, "A1").click();
    await app.page.keyboard.type("100", { delay: 20 });
    await app.page.keyboard.press("Enter");
    await until(async () => (await cellOf(app, "A3"))?.value === "130", { what: "A3 = 130" });
    await app.menu("edit.undo");
    await until(async () => (await cellOf(app, "A3"))?.value === "42", { what: "A3 back to 42" });
    assert.equal((await cellOf(app, "A3")).source, "= sum({A1:A2})", "the formula is untouched");
  } finally {
    await app.close();
  }
});

await check("fill down shifts relative references", async () => {
  const app = await open(board());
  try {
    await dismissWelcome(app.page);
    await app.call("set_cell", { name: "A1", content: "1" });
    await app.call("set_cell", { name: "A2", content: "2" });
    await app.call("set_cell", { name: "A3", content: "3" });
    await cell(app.page, "B1").click();
    await app.page.keyboard.type("= {A1} * 10", { delay: 10 });
    await app.page.keyboard.press("Enter");
    await cell(app.page, "B1").click();
    await cell(app.page, "B3").click({ modifiers: ["Shift"] });
    await app.menu("edit.fill_down");
    await until(async () => (await cellOf(app, "B3"))?.value === "30", { what: "B3 = 30" });
    assert.equal((await cellOf(app, "B3")).source, "= {A3} * 10");
  } finally {
    await app.close();
  }
});

await check("verify, flag with a note, and the flag lands in Needs You", async () => {
  const app = await open(board());
  try {
    await dismissWelcome(app.page);
    await app.call("set_cell", { name: "total", content: "42" });
    await app.call("set_cell", { name: "A1", content: "7" });
    await cell(app.page, "A1").click();
    await app.menu("cell.verify");
    await until(async () => (await cellOf(app, "A1"))?.mark?.kind === "verified", { what: "verified" });
    await app.menu("cell.flag");
    await app.page.waitForSelector("#overlay:not([hidden])");
    await app.page.keyboard.type("check the total");
    await app.page.keyboard.press("Enter");
    await until(async () => (await cellOf(app, "A1"))?.mark?.note === "check the total", { what: "the flag note" });
    await app.page.click("#tab-decisions");
    await until(async () => /check the total/i.test(await app.page.textContent("#tray")), { what: "the flag in Needs You" });
  } finally {
    await app.close();
  }
});

await check("an agent's question is answered from its Needs You row", async () => {
  const app = await open(board());
  const mcp = spawn(recalc, ["mcp", "--id", "probe"], { cwd: app.dir, stdio: ["pipe", "pipe", "ignore"] });
  try {
    await dismissWelcome(app.page);
    let out = "";
    mcp.stdout.on("data", (d) => (out += d));
    const send = (m) => mcp.stdin.write(JSON.stringify(m) + "\n");
    send({ jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2025-06-18", clientInfo: { name: "probe" } } });
    send({ jsonrpc: "2.0", id: 2, method: "tools/call", params: { name: "ask_cell", arguments: { name: "deploy_ok", question: "Deploy to staging now?", options: ["yes", "no"], recommended: 1, timeout_ms: 30000 } } });
    await app.page.click("#tab-decisions");
    await until(async () => (await app.page.textContent("#tray")).includes("Deploy to staging"), { what: "the question" });
    await app.shot("question");
    await app.page.locator("#tray .need-ask .need-acts button", { hasText: "1 yes" }).click();
    await until(() => out.includes('\\"resolved\\":true') || out.includes('"resolved":true'), { what: "the agent to hear yes", timeout: 10000 });
    assert.ok(out.includes("yes"));
  } finally {
    mcp.kill();
    await app.close();
  }
});

await check("a headless Claude run that wants Bash asks in Needs You, and Allow lets it finish", async () => {
  const tools = join(here, ".tools");
  mkdirSync(tools, { recursive: true });
  const fake = join(tools, "claude");
  const recording = join(here, "../../tests/fixtures/agents/claude-2.1.280/stream-permission-allowed.jsonl");
  writeFileSync(
    fake,
    `#!/bin/sh\nread -r line\nwhile IFS= read -r ev <&3; do\n  printf '%s\\n' "$ev"\n  case "$ev" in *'"control_request"'*) read -r line; echo "$line" > ${join(tools, "answer.json")};; esac\ndone 3< ${recording}\n`,
  );
  chmodSync(fake, 0o755);
  const dir = board();
  mkdirSync(join(dir, ".recalc"), { recursive: true });
  writeFileSync(join(dir, ".recalc/config.toml"), `[paths]\nclaude = "${fake}"\n`);
  const app = await open(dir);
  try {
    await dismissWelcome(app.page);
    await app.call("set_cell", { name: "A1", content: "@claude('make a directory')" });
    await cell(app.page, "A1").click();
    await app.menu("cell.run");
    await app.page.click("#tab-decisions");
    await until(async () => (await app.page.textContent("#tray")).includes("wants to use Bash: mkdir probe-dir"), { what: "the permission question" });
    await app.shot("permission");
    await app.page.locator("#tray .need-ask .need-acts button", { hasText: "Allow once" }).click();
    await until(async () => (await cellOf(app, "A1"))?.value === "done", { what: "A1 = done" });
    assert.ok(existsSync(join(tools, "answer.json")), "the answer reached the agent");
  } finally {
    await app.close();
  }
});

await check("a narrow window keeps the side panel on screen", async () => {
  const app = await open(board("legal"), { width: 780, height: 640 });
  try {
    const overflow = await app.page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth);
    const right = await app.page.evaluate(() => document.querySelector("#side").getBoundingClientRect().right);
    await app.shot("narrow");
    assert.equal(overflow, false, "no sideways scroll");
    assert.ok(right <= 780, `side panel ends at ${right}`);
  } finally {
    await app.close();
  }
});

/** The legal demo's contracts, with a `claude` whose runs hit a usage limit
 * and whose live sessions idle. */
function contractsBoard() {
  const tools = join(here, ".tools");
  mkdirSync(tools, { recursive: true });
  const fake = join(tools, "claude-limited");
  writeFileSync(
    fake,
    `#!/bin/sh\ncase "$*" in *stream-json*) cat ${join(here, "../../tests/fixtures/agents/claude-2.1.280/rate-limit.synthetic.json")} ;; *) echo "fake claude session ready"; sleep 600 ;; esac\n`,
  );
  chmodSync(fake, 0o755);
  const dir = board();
  cpSync(join(here, "../../demos/legal/contracts"), join(dir, "contracts"), { recursive: true });
  mkdirSync(join(dir, ".recalc"), { recursive: true });
  writeFileSync(join(dir, ".recalc/config.toml"), `max_agents = 1\n[paths]\nclaude = "${fake}"\n`);
  return dir;
}

await check("a document grid from a folder: board-relative paths, a typed question column, and export", async () => {
  const dir = contractsBoard();
  const app = await open(dir);
  try {
    await dismissWelcome(app.page);
    await app.pick(join(dir, "contracts"));
    await app.menu("file.new_board");
    await app.page.locator(".start-pane:not([hidden]) .card", { hasText: "Document grid from a folder" }).click();
    await app.page.waitForSelector("#overlay:not([hidden]) .qcol-question", { timeout: 10000 });
    const docs = (await app.call("get_board")).cells.filter((c) => /^A([2-9]|\d\d)$/.test(c.name));
    assert.equal(docs.length, 7);
    for (const d of docs) assert.match(d.value, /^contracts\/[\w-]+\.md$/, "paths are relative to the board");
    await app.page.fill(".qcol-question", "Can either party assign the agreement without consent?");
    await app.page.locator(".qcol-kinds button").filter({ hasText: /yes/i }).first().click();
    await app.page.locator(".qcol-foot button", { hasText: "Add column" }).click();
    await until(async () => (await cellOf(app, "B2"))?.answer_type?.kind === "yesno", { what: "B2 typed yes/no" });
    assert.match((await cellOf(app, "B2")).source, /^@claude\(/);
    const csv = join(dir, "out.csv");
    await app.pick(csv);
    await app.menu("file.export_csv");
    await until(() => existsSync(csv), { what: "the csv" });
    const xlsx = join(dir, "out.xlsx");
    await app.pick(xlsx);
    await app.menu("file.export_xlsx");
    await until(() => existsSync(xlsx), { what: "the workbook" });
  } finally {
    await app.close();
  }
});

await check("a question column asks about the column you pick: documents in B are read as documents, IDs in A as the row's subject", async () => {
  const dir = board();
  mkdirSync(join(dir, "contracts"), { recursive: true });
  writeFileSync(join(dir, "contracts/acme.md"), "Acme agreement");
  writeFileSync(join(dir, "contracts/globex.md"), "Globex agreement");
  mkdirSync(join(dir, ".recalc"), { recursive: true });
  appendFileSync(join(dir, ".recalc/config.toml"), '\n[models]\nfast = "claude-haiku-5-5"\n');
  const app = await open(dir);
  try {
    await dismissWelcome(app.page);
    for (const [name, content] of [["A1", "ID"], ["A2", "C-1"], ["A3", "C-2"], ["B1", "Document"], ["B2", "contracts/acme.md"], ["B3", "contracts/globex.md"]]) {
      await app.call("set_cell", { name, content });
    }
    const ask = async (question, aboutCol, model) => {
      await app.menu("cell.add_column");
      await app.page.waitForSelector("#overlay:not([hidden]) .qcol-question", { timeout: 10000 });
      const picker = app.page.locator(".qcol-about-col");
      if (aboutCol) await picker.selectOption(aboutCol);
      if (model) await app.page.fill(".qcol-model", model);
      const shown = await picker.locator("option:checked").textContent();
      assert.match(await app.page.locator(".qcol-foot label.check").first().textContent(), /2 agent runs/, "the run count follows the picked column");
      await app.page.fill(".qcol-question", question);
      await app.page.locator(".qcol-foot input[type=checkbox]").first().uncheck();
      await app.page.locator(".qcol-foot button", { hasText: "Add column" }).click();
      await app.page.waitForSelector("#overlay", { state: "hidden" });
      return shown;
    };
    assert.equal(await ask("Who are the parties?"), "B · Document", "the column of documents is picked for you");
    await until(async () => (await cellOf(app, "C2"))?.source, { what: "C2's prompt" });
    assert.match((await cellOf(app, "C2")).source, /\{instructions\} Question: \{C\$1\}.*Document: \{\$B2\}/);
    assert.match((await cellOf(app, "C3")).source, /Document: \{\$B3\}/);

    assert.equal(await ask("Which region is this customer in?", "1"), "A · ID");
    await until(async () => (await cellOf(app, "D2"))?.source, { what: "D2's prompt" });
    const d2 = (await cellOf(app, "D2")).source;
    assert.match(d2, /Question: \{D\$1\}.*About: \{\$A2\}/, "IDs are the row's subject");
    assert.doesNotMatch(d2, /instructions|Document:/, "not read as a document");
    assert.doesNotMatch(d2, /model=/, "no model picked: the agent's default");

    // A model for the column: the board's aliases are offered.
    await app.menu("cell.add_column");
    await app.page.waitForSelector("#overlay:not([hidden]) .qcol-model", { timeout: 10000 });
    assert.deepEqual(await app.page.$$eval("#qcol-models option", (o) => o.map((x) => x.value)), ["fast"]);
    await app.page.keyboard.press("Escape");
    await app.page.waitForSelector("#overlay", { state: "hidden" });
    await ask("When does it renew?", "2", "fast");
    await until(async () => (await cellOf(app, "E2"))?.source, { what: "E2's prompt" });
    assert.match((await cellOf(app, "E2")).source, /", model='fast'\)$/);
  } finally {
    await app.close();
  }
});

await check("a citation to a passage the file lacks is flagged on the cell, in the Inspector, and in Needs You", async () => {
  const dir = contractsBoard();
  const app = await open(dir);
  const mcp = spawn(recalc, ["mcp", "--id", "probe"], { cwd: dir, stdio: ["pipe", "pipe", "ignore"] });
  try {
    await dismissWelcome(app.page);
    let out = "";
    mcp.stdout.on("data", (d) => (out += d));
    const send = (m) => mcp.stdin.write(JSON.stringify(m) + "\n");
    send({ jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2025-06-18", clientInfo: { name: "probe" } } });
    send({ jsonrpc: "2.0", id: 2, method: "tools/call", params: { name: "set_cell", arguments: { name: "C3", content: "Texas", cite: { file: "contracts/globex-services-agreement.md", quote: "governed by the laws of the State of Texas" } } } });
    await until(() => out.includes('"id":2'), { what: "the write" });
    assert.ok(out.includes("cite_not_found"), "the agent is told");
    await app.page.click("#tab-decisions");
    await until(async () => /quoted passage isn't in/i.test(await app.page.textContent("#tray")), { what: "the Needs You row" });
    await until(async () => (await app.page.locator("#cells .cell[data-name='C3'] .cite-mark.missing").count()) === 1, { what: "the red mark" });
    await cell(app.page, "C3").click();
    await until(async () => (await app.page.textContent("#inspector-body")).includes("isn't in"), { what: "the Inspector warning" });
  } finally {
    mcp.kill();
    await app.close();
  }
});

await check("a usage limit holds the queue, and the status bar says until when", async () => {
  const dir = contractsBoard();
  const app = await open(dir);
  try {
    await dismissWelcome(app.page);
    // One run selection, not three cell runs: running a cell by hand lifts the hold.
    for (const n of ["A2", "A3", "A4"]) await app.call("set_cell", { name: n, content: `@claude('${n}')` });
    await cell(app.page, "A2").click();
    await cell(app.page, "A4").click({ modifiers: ["Shift"] });
    await app.menu("cell.run_selection");
    await until(async () => /Agents held until .*usage limit/.test(await app.page.textContent("#status-agents")), { what: "the hold on the status bar" });
    assert.equal((await cellOf(app, "A3")).state, "queued", "the next one waits instead of failing");
    assert.equal((await cellOf(app, "A4")).state, "queued");
  } finally {
    await app.close();
  }
});

await check("a live session opens in the Inspector and ends", async () => {
  const dir = contractsBoard();
  const app = await open(dir);
  try {
    await dismissWelcome(app.page);
    await app.call("set_cell", { name: "E2", content: "@claude('talk to me')" });
    await cell(app.page, "E2").click();
    await app.menu("agent.session");
    await until(async () => ((await app.page.locator("#inspector .xterm-rows").textContent()) ?? "").includes("fake claude session ready"), { what: "the session's output", timeout: 15000 });
    assert.equal((await cellOf(app, "E2_status")).value, "running");
    await app.page.locator("#inspector button", { hasText: /End session/i }).first().click();
    await until(async () => (await cellOf(app, "E2_status"))?.value === "ended", { what: "ended" });
  } finally {
    await app.close();
  }
});

const failed = results.filter((r) => r[1] !== "ok");
console.log(`\n${results.length - failed.length} of ${results.length} passed`);
process.exit(failed.length ? 1 : 0);
