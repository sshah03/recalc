// Does the Driver use formulas where a spreadsheet person would? It's asked
// for a table of orders with totals, counts, a flag column and a tax column
// over a named rate, as live formulas. The check finds them by what they do,
// grades their values, then changes an amount and the rate and checks that
// everything that should follow did. Paid (one driver session, efficient
// models). `node formulas-driver.mjs [claude|codex]`
import { mkdirSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { board, open, until } from "./lib.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const driver = process.argv[2] ?? "claude";
const log = [];
const note = (s) => {
  log.push(s);
  console.log("•", s);
};

const ORDERS = [
  ["O-101", "Acme", "Europe", 1250],
  ["O-102", "Globex", "Asia", 480],
  ["O-103", "Initech", "Europe", 320.5],
  ["O-104", "Umbrella", "North America", 2100],
  ["O-105", "Hooli", "Europe", 999.99],
  ["O-106", "Stark", "Asia", 1000],
  ["O-107", "Wayne", "North America", 75],
  ["O-108", "Wonka", "Europe", 1500],
];
const REQUEST =
  "documents/orders.csv has this quarter's orders. Put them on the board: one row per order with Order, Customer, Region and Amount (a number). " +
  "Then add these as live formulas that stay right when I change a number, not values you work out yourself: the total of Amount under the table, " +
  "the average order, how many orders are from Europe, and the total for Europe. Add a column that says High for orders of 1000 or more and Normal otherwise. " +
  "Put the sales tax rate, 8%, in a named cell called tax_rate, and add a column with each order's amount including tax that uses it.";

// ---- the board ----
const dir = board();
mkdirSync(join(dir, "documents"), { recursive: true });
writeFileSync(join(dir, "documents/orders.csv"), ["Order,Customer,Region,Amount", ...ORDERS.map((o) => o.join(","))].join("\n") + "\n");
mkdirSync(join(dir, ".recalc"), { recursive: true });
writeFileSync(join(dir, ".recalc/config.toml"), `driver = "${driver}"\n`);
note(`board ${dir}, driver ${driver}`);

const app = await open(dir);
const { page } = app;
const shot = (n) => app.shot(`formulas-${driver}-${n}`);
const welcome = await page.waitForSelector("#overlay:not([hidden])", { timeout: 5000 }).catch(() => null);
if (welcome) {
  await page.keyboard.press("Escape");
  await page.waitForSelector("#overlay", { state: "hidden" });
}

// ---- the driver ----
const screen = async () => ((await page.locator("#tray .xterm-rows").textContent().catch(() => "")) ?? "").replace(/\s+/g, " ");
await page.click("#tab-driver");
await page.locator("#tray button", { hasText: "Start" }).click();
await until(async () => (await page.locator("#tray .xterm").count()) > 0, { timeout: 30000, what: "the driver's terminal" });
const t0 = Date.now();
while (Date.now() - t0 < 25000) {
  const s = await screen();
  if (/Update available/i.test(s) && /Skip/.test(s)) {
    await page.locator("#tray .xterm").click();
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("Enter");
    note("skipped Codex's update prompt");
    await page.waitForTimeout(2000);
    continue;
  }
  if (/trust this folder/i.test(s)) {
    await page.locator("#tray .xterm").click();
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("Enter");
    break;
  }
  if (/trust the (files|contents)|Do you trust/i.test(s)) {
    await page.locator("#tray .xterm").click();
    await page.keyboard.press("Enter");
    break;
  }
  if (/[>❯]\s*$|Type your message|Ask Codex|▌/i.test(s) && Date.now() - t0 > 8000) break;
  await page.waitForTimeout(1000);
}
await page.waitForTimeout(4000);
if (/Update available|Update now/i.test(await screen())) {
  note("the driver is showing an update prompt; stopping");
  await app.close();
  process.exit(1);
}

async function answerAsThePerson(b) {
  for (const c of b.cells.filter((x) => x.decision)) {
    const labels = (c.decision.options ?? []).map((o) => o.label);
    const forRun = labels.findIndex((l) => /^Allow .* for this run$/.test(l));
    const i = /wants to use/i.test(c.decision.question) ? (forRun >= 0 ? forRun : labels.indexOf("Allow once")) : (c.decision.recommended ?? 0);
    const args = labels.length ? { name: c.name, index: i } : { name: c.name, text: "Use your best judgment." };
    await app.call("resolve_decision", args).catch(() => {});
    note(`answered ${c.name}: ${c.decision.question.slice(0, 80)}`);
  }
}
async function settle(quiet = 60000, timeout = 20 * 60000) {
  const start = Date.now();
  let last = "";
  let since = Date.now();
  for (;;) {
    const b = await app.call("get_board");
    await answerAsThePerson(b);
    const running = b.cells.filter((c) => c.state === "computing" || c.state === "queued").length;
    const sig = JSON.stringify(b.cells.map((c) => [c.name, c.value, c.state, c.source]));
    const working = /esc to interrupt|Working \(|Thinking/i.test(await screen());
    if (sig !== last || running || working) {
      last = sig;
      since = Date.now();
    }
    if (Date.now() - since > quiet || Date.now() - start > timeout) break;
    await page.waitForTimeout(5000);
  }
  note(`settled after ${Math.round((Date.now() - start) / 1000)}s`);
}

await page.locator("#tray .xterm").click();
await page.keyboard.type(REQUEST, { delay: 5 });
await page.waitForTimeout(1500);
await page.keyboard.press("Enter");
await page.waitForTimeout(3000);
if (driver === "codex" && (await screen()).includes(REQUEST.slice(-40).replace(/\s+/g, " "))) await page.keyboard.press("Enter");
await page.waitForTimeout(20000);
await settle();
await shot("done");

// ---- grading ----
const results = [];
const ok = (what, pass, detail = "") => {
  results.push({ what, pass, detail });
  note(`${pass ? "ok  " : "FAIL"} ${what}${detail ? ` — ${detail}` : ""}`);
};
const close = (a, b, tol = 0.01) => Math.abs(Number(a) - b) <= tol;
const num = (v) => Number(String(v).replace(/[$,]/g, ""));

async function grade(phase, amounts, rate) {
  const b = await app.call("get_board");
  const exprs = b.cells.filter((c) => c.kind === "expr");
  const has = (re) => exprs.filter((c) => re.test(c.source ?? ""));
  const total = amounts.reduce((s, a) => s + a, 0);
  const eu = ORDERS.map((o, i) => [o[2], amounts[i]]).filter(([r]) => r === "Europe");
  const t = has(/\bsum\s*\(/i).find((c) => close(num(c.value), total));
  ok(`${phase}: a sum formula gives the total ${total.toFixed(2)}`, !!t, t ? `${t.name} = ${t.source}` : `sums: ${has(/\bsum\s*\(/i).map((c) => `${c.name}=${c.value}`).join(", ")}`);
  const a = has(/\b(avg|average)\s*\(/i).find((c) => close(num(c.value), total / amounts.length));
  ok(`${phase}: an avg formula gives the average`, !!a, a ? `${a.name} = ${a.source}` : "");
  const n = has(/countifs?\s*\(/i).find((c) => num(c.value) === eu.length);
  ok(`${phase}: a countif counts ${eu.length} European orders`, !!n, n ? `${n.name} = ${n.source}` : "");
  const s = has(/sumif\s*\(/i).find((c) => close(num(c.value), eu.reduce((x, [, v]) => x + v, 0)));
  ok(`${phase}: a sumif totals Europe`, !!s, s ? `${s.name} = ${s.source}` : "");
  const flags = has(/\bif\s*\(/i).filter((c) => /^(High|Normal)$/.test(c.value));
  const wantHigh = amounts.filter((x) => x >= 1000).length;
  ok(`${phase}: an if column flags ${wantHigh} High of ${amounts.length}`, flags.length >= amounts.length && flags.filter((c) => c.value === "High").length === wantHigh, `${flags.length} if cells, ${flags.filter((c) => c.value === "High").length} High`);
  const taxed = has(/tax_rate/).filter((c) => !/^\s*=\s*[\d.]+\s*$/.test(c.source ?? ""));
  const right = amounts.filter((x) => taxed.some((c) => close(num(c.value), x * (1 + rate), 0.011))).length;
  ok(`${phase}: a tax column uses tax_rate and is right for all ${amounts.length} orders`, right === amounts.length, `${taxed.length} cells use tax_rate, ${right} match`);
  return b;
}

const amounts = ORDERS.map((o) => o[3]);
const b = await grade("as asked", amounts, 0.08);
const rate = b.cells.find((c) => c.name === "tax_rate");
// As formulas read it: "8%" is 0.08.
const asNumber = (v) => {
  const t = String(v).trim().replace(/[$,]/g, "");
  return t.endsWith("%") ? Number(t.slice(0, -1)) / 100 : Number(t);
};
ok("tax_rate is a named cell that reads as 8%", !!rate && close(asNumber(rate.value), 0.08, 1e-9), rate ? `tax_rate = ${JSON.stringify(rate.value)}` : "missing");

// Change an amount and the rate, as a person would: everything should follow.
const row = b.cells.find((c) => /^[A-Z]+\d+$/.test(c.name) && c.value === "O-107")?.name.replace(/^[A-Z]+/, "");
const header = b.cells.find((c) => /^[A-Z]+1$/.test(c.name) && /amount/i.test(c.value) && !/tax/i.test(c.value));
if (row && header) {
  const amountCell = header.name.replace(/1$/, "") + row;
  await app.call("set_cell", { name: amountCell, content: "1075" });
  if (rate) await app.call("set_cell", { name: "tax_rate", content: "0.1" });
  await page.waitForTimeout(2000);
  const changed = amounts.slice();
  changed[6] = 1075;
  await grade(`after ${amountCell} = 1075 and tax_rate = 0.1`, changed, rate ? 0.1 : 0.08);
} else ok("found O-107's amount to change", false, `row ${row}, header ${header?.name}`);

const cells = (await app.call("get_board")).cells;
writeFileSync(join(here, `shots/formulas-${driver}-report.json`), JSON.stringify({ driver, dir, log, results, cells: cells.map((c) => ({ name: c.name, kind: c.kind, source: c.source, value: c.value, by: c.provenance?.by })) }, null, 2));
await app.close();
const failed = results.filter((r) => !r.pass).length;
console.log(`\n${results.length - failed} of ${results.length} passed`);
process.exit(failed ? 1 : 0);
