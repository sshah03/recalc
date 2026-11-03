// The recent features with a real agent, Claude or Codex, on efficient models:
// a document grid over a folder outside the board, a JSON answer type, a
// flagged answer re-run with the reviewer's note, formulas waiting on answers,
// and a duplicated sheet answering from its own inputs. Paid (about fifteen
// one-question runs). `node recent.mjs [claude|codex]`
import assert from "node:assert/strict";
import { cpSync, mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { board, open, until, recalc } from "./lib.mjs";
import { execFileSync } from "node:child_process";

const agent = process.argv[2] ?? "claude";
const results = [];
async function check(name, f) {
  const t0 = Date.now();
  try {
    await f();
    results.push([name, "ok"]);
    console.log(`ok   ${name} (${((Date.now() - t0) / 1000).toFixed(0)}s)`);
  } catch (e) {
    results.push([name, "FAIL"]);
    console.log(`FAIL ${name}\n     ${String(e.message ?? e).split("\n").slice(0, 6).join("\n     ")}`);
  }
}

// Contracts in a folder that isn't the board's, as a person's documents are.
const demo = mkdtempSync(join(tmpdir(), "recalc-recent-demo-"));
execFileSync(recalc, ["demo", "legal", "--into", join(demo, "b"), "--no-open"]);
const docs = mkdtempSync(join(tmpdir(), "recalc-recent-docs-"));
for (const f of ["globex-services-agreement.md", "initech-mutual-nda.md", "umbrella-saas-subscription.md"]) {
  cpSync(join(demo, "b/contracts", f), join(docs, f));
}
const LAW = { "globex-services-agreement.md": /new york/i, "initech-mutual-nda.md": /california/i, "umbrella-saas-subscription.md": /england/i };

const dir = board();
mkdirSync(join(dir, ".recalc"), { recursive: true });
writeFileSync(join(dir, ".recalc/config.toml"), "max_agents = 4\n");
const app = await open(dir);
const { page } = app;
if (await page.waitForSelector("#overlay:not([hidden])", { timeout: 5000 }).catch(() => null)) await page.keyboard.press("Escape");
const all = async () => Object.fromEntries((await app.call("get_board")).cells.map((c) => [c.name, c]));
const settled = async (names) => {
  const cells = await all();
  return names.every((n) => cells[n] && (typeof cells[n].state !== "string" || cells[n].state === "ok") && cells[n].value !== "" && !cells[n].decision);
};
/** Questions the agents put to the person while the runs went. */
const asked = async () => Object.values(await all()).filter((c) => c.decision).map((c) => `${c.name}: ${c.decision.question.slice(0, 80)}`);
const ask = (q, ref) => `@${agent}('${q} Document: {${ref}}')`;

await check("a document grid over a folder outside the board reads its documents without asking", async () => {
  await app.pick(docs);
  await app.menu("file.new_board");
  await page.locator(".start-pane:not([hidden]) .card", { hasText: "Document grid from a folder" }).click();
  await page.waitForSelector("#overlay:not([hidden]) .qcol-question", { timeout: 10000 });
  await page.fill(".qcol-question", "Which jurisdiction's law governs this agreement? Answer with the state or country only.");
  await page.locator(".qcol-agent").selectOption(agent);
  await page.locator(".qcol-foot button", { hasText: "Add column" }).click();
  await page.waitForSelector("#overlay", { state: "hidden" });
  const names = ["B2", "B3", "B4"];
  let questions = [];
  await until(async () => {
    questions = questions.concat(await asked());
    return settled(names);
  }, { timeout: 600000, every: 3000, what: "the governing-law column" });
  const cells = await all();
  for (const n of names) {
    const file = cells[`A${n.slice(1)}`].value.split("/").pop();
    assert.match(cells[n].value, LAW[file], `${n} (${file}): ${cells[n].value}`);
  }
  assert.deepEqual([...new Set(questions)], [], "no one was asked for permission to read");
});

// The rest go on a plain sheet of the new board.
await app.call("set_cell", { name: "Work!A1", content: join(docs, "umbrella-saas-subscription.md") });

await check("a formula over an answer that isn't in yet waits instead of erroring", async () => {
  await app.call("set_cell", { name: "Work!B1", content: ask("How long is the subscription term? Answer in one short phrase.", "A1") });
  // Arithmetic on an empty answer would fail; it waits for the answer instead.
  await app.call("set_cell", { name: "Work!C1", content: "= {B1} - 1" });
  const c1 = (await all())["Work!C1"];
  assert.equal(c1.state, "stale", `C1 is ${JSON.stringify(c1.state)}`);
});

await check("a JSON answer type holds the answer to an object with its keys", async () => {
  await app.call("set_cell", { name: "Work!B2", content: ask("Give the agreement's parties and governing law.", "A1") });
  await app.call("set_answer_type", { names: ["Work!B2"], kind: "json", options: ["parties", "governing_law"] });
  await app.call("trigger", { name: "Work!B2" });
  await until(() => settled(["Work!B2"]), { timeout: 300000, every: 3000, what: "B2" });
  const v = JSON.parse((await all())["Work!B2"].value);
  assert.ok(v.parties && v.governing_law, JSON.stringify(v));
  assert.match(JSON.stringify(v.governing_law), /england/i);
});

await check("a flagged answer re-run takes the reviewer's note", async () => {
  await app.call("set_cell", { name: "Work!B3", content: ask("Which law governs this agreement? Answer with the jurisdiction only.", "A1") });
  await app.call("trigger", { name: "Work!B3" });
  await until(() => settled(["Work!B3"]), { timeout: 300000, every: 3000, what: "B3" });
  const first = (await all())["Work!B3"].value;
  assert.match(first, /england/i, first);
  await app.call("set_mark", { names: ["Work!B3"], kind: "flagged", note: "Answer with the ISO 3166-1 alpha-2 country code of that jurisdiction's country, nothing else." });
  await app.call("trigger", { name: "Work!B3" });
  await until(async () => (await settled(["Work!B3"])) && (await all())["Work!B3"].value !== first, { timeout: 300000, every: 3000, what: "B3 re-answered" });
  const second = (await all())["Work!B3"].value.trim();
  assert.match(second, /^"?GB"?\.?$/i, `after the note: ${second}`);
});

await check("a duplicated sheet answers from its own inputs", async () => {
  await app.call("sheet_edit", { action: "duplicate", name: "Work", to: "Work 2" });
  await app.call("set_cell", { name: "'Work 2'!A1", content: join(docs, "initech-mutual-nda.md") });
  const copy = (await all())["Work 2!B3"];
  assert.equal(copy.value, "", "the copy starts without the answer");
  await app.call("set_mark", { names: ["Work 2!B3"], kind: null });
  await app.call("trigger", { name: "Work 2!B3" });
  await until(() => settled(["Work 2!B3"]), { timeout: 300000, every: 3000, what: "the copy's B3" });
  assert.match((await all())["Work 2!B3"].value, /california/i);
});

await app.close();
console.log(`${results.filter((r) => r[1] === "ok").length} of ${results.length} passed`);
process.exit(results.some((r) => r[1] === "FAIL") ? 1 : 0);
