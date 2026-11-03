// Runs a demo's whole table through the UI and reports each agent cell's
// answer, state, citation and cost, plus the summary cells.
// `node demo-full.mjs legal`. Paid (real agent runs).
import { writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { board, open, cell, until } from "./lib.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const demo = process.argv[2] ?? "legal";
const app = await open(board(demo));
const { page } = app;
const agents = (await app.call("get_board")).cells.filter((c) => c.kind === "agent_call" && /^[A-Z]+\d+$/.test(c.name));
const names = agents.map((c) => c.name).sort();
const first = names[0];
const last = names[names.length - 1];
console.log(`${demo}: ${names.length} agent cells, ${first}:${last}`);
// Prefer the Inspector's Select button: the block's far corner may be off
// screen, where the virtual grid draws no cell to shift-click.
const pick = page.locator("#inspector-body button", { hasText: /^Select / });
if (await pick.count()) await pick.first().click();
else {
  await cell(page, first).click();
  await cell(page, last).click({ modifiers: ["Shift"] });
}
console.log(`selected: ${await page.textContent("#namebox")}`);
await app.menu("cell.run_selection");
// A big run asks for confirmation first.
const go = page.locator("#overlay:not([hidden]) button", { hasText: /^Run \d+$/ });
if (await go.count()) await go.first().click();
const t0 = Date.now();
await until(async () => {
  const b = await app.call("get_board");
  const busy = b.cells.filter((c) => c.kind === "agent_call" && (c.state === "computing" || c.state === "queued")).length;
  process.stdout.write(`\r${Math.round((Date.now() - t0) / 1000)}s: ${busy} running or queued   `);
  return busy === 0;
}, { timeout: 60 * 60 * 1000, every: 5000, what: "the table" });
console.log();
// Named agent cells summarise the grid, so they run after it.
const named = (await app.call("get_board")).cells.filter((c) => c.kind === "agent_call" && !/^[A-Z]+\d+$/.test(c.name));
for (const n of named) {
  await app.call("trigger", { name: n.name }).catch(() => {});
}
if (named.length) {
  await until(async () => (await app.call("get_board")).cells.filter((c) => c.kind === "agent_call" && (c.state === "computing" || c.state === "queued")).length === 0, { timeout: 15 * 60 * 1000, every: 5000, what: "the named agent cells" });
}
await page.waitForTimeout(1500);
await app.shot(`${demo}-full`);
const b = await app.call("get_board");
const report = {
  demo,
  seconds: Math.round((Date.now() - t0) / 1000),
  cost: b.cells.reduce((s, c) => s + (c.usage?.cost_usd ?? 0), 0),
  cells: b.cells
    .filter((c) => c.kind === "agent_call")
    .map((c) => ({ name: c.name, value: c.value, state: c.state, cite: c.cite ? { file: c.cite.file, found: c.cite.found } : null, cost: c.usage?.cost_usd ?? null })),
  headers: Object.fromEntries(b.cells.filter((c) => /^[A-Z]+1$/.test(c.name)).map((c) => [c.name, c.value])),
  documents: Object.fromEntries(b.cells.filter((c) => /^A\d+$/.test(c.name)).map((c) => [c.name, c.value])),
  summaries: Object.fromEntries(b.cells.filter((c) => !/^[A-Z]+\d+$/.test(c.name) && c.kind !== "agent_call").map((c) => [c.name, { value: c.value, state: c.state, source: c.source }])),
  errors: app.errors,
};
const out = join(here, `shots/${demo}-report.json`);
writeFileSync(out, JSON.stringify(report, null, 2));
const errored = report.cells.filter((c) => typeof c.state === "object");
const uncited = report.cells.filter((c) => c.state === "ok" && !c.cite);
const lost = report.cells.filter((c) => c.cite && c.cite.found === false);
console.log(`done in ${report.seconds}s, $${report.cost.toFixed(2)}: ${report.cells.length} cells, ${errored.length} errored, ${uncited.length} uncited, ${lost.length} citations not found`);
for (const c of errored) console.log(`  ${c.name}: ${JSON.stringify(c.state)}`);
console.log(`summaries: ${JSON.stringify(report.summaries)}`);
console.log(`report: ${out}`);
await app.close();
