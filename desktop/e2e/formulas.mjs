// Formulas in the real app: every function with its edge cases, named cells
// and ranges, other sheets, $ anchors through fill, references to deleted
// cells, cycles, and building one by typing and clicking. Free.
// `cargo build -p recalc-desktop`, then `node formulas.mjs [filter]`.
import assert from "node:assert/strict";
import { board, cell, open, until } from "./lib.mjs";

process.setMaxListeners(50);
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
    console.log(`FAIL ${name}\n     ${String(e.stack ?? e.message ?? e).split("\n").slice(0, 8).join("\n     ")}`);
  }
}

/** Numbers in A2:A5 (A6 empty), regions in B2:B5, amounts in C2:C5. */
const DATA = [
  ["A2", "10"], ["A3", "20"], ["A4", "30"], ["A5", "40"],
  ["B2", "Europe"], ["B3", "Asia"], ["B4", "Europe"], ["B5", "N/A"],
  ["C2", "100.5"], ["C3", "200"], ["C4", "300.25"], ["C5", "50"],
];
async function start() {
  const app = await open(board());
  const shown = await app.page.waitForSelector("#overlay:not([hidden])", { timeout: 4000 }).catch(() => null);
  if (shown) {
    await app.page.keyboard.press("Escape");
    await app.page.waitForSelector("#overlay", { state: "hidden" });
  }
  for (const [n, v] of DATA) await app.call("set_cell", { name: n, content: v });
  return app;
}
const get = async (app, n) => (await app.call("get_board")).cells.find((c) => c.name === n);
const value = async (app, n) => (await get(app, n))?.value ?? "";
const error = async (app, n) => {
  const s = (await get(app, n))?.state;
  return s && typeof s === "object" ? s.error : null;
};
/** Set `cases` into column F and compare each value (or error) to what's wanted. */
async function table(app, cases) {
  const wrong = [];
  for (const [i, [src]] of cases.entries()) await app.call("set_cell", { name: `F${i + 2}`, content: src });
  for (const [i, [src, want]] of cases.entries()) {
    const n = `F${i + 2}`;
    const got = want instanceof RegExp ? (await error(app, n)) ?? `value ${await value(app, n)}` : await value(app, n);
    const fine = want instanceof RegExp ? want.test(got) : got === want;
    if (!fine) wrong.push(`${src}  →  ${JSON.stringify(got)}, wanted ${want}`);
  }
  assert.deepEqual(wrong, [], `${wrong.length} of ${cases.length} wrong:\n${wrong.join("\n")}`);
}

await check("every function gives what it says, including empty cells and text in a range", async () => {
  const app = await start();
  try {
    await table(app, [
      ["= sum({A2:A6})", "100"],
      ["= sum({A2:A3}, 5, {A4})", "65"],
      ["= avg({A2:A6})", "25"],
      ["= count({A2:A6})", "4"],
      ["= count({B2:B6})", "4"],
      ["= min({A2:A6})", "10"],
      ["= max({A2:A6})", "40"],
      ["= median({A2:A6})", "25"],
      ["= median({A2:A4})", "20"],
      ["= round(2.5)", "3"],
      ["= round(-2.5)", "-3"],
      ["= round(1.2345, 2)", "1.23"],
      ["= round(2.345, 2)", "2.35"],
      ["= abs(-3)", "3"],
      ["= countif({B2:B6}, 'Europe')", "2"],
      ["= countif({A2:A6}, '>15')", "3"],
      ["= countif({B2:B6}, '<>Europe')", "3"],
      ["= countifs({B2:B6}, 'Europe', {A2:A6}, '>15')", "1"],
      ["= sumif({B2:B6}, 'Europe', {C2:C6})", "400.75"],
      ["= sumif({A2:A6}, '>15')", "90"],
      ["= if({A2} > 5, 'big', 'small')", "big"],
      ["= if({A2} > 50, 'big', 'small')", "small"],
      ["= and(true, false)", "false"],
      ["= or(true, false)", "true"],
      ["= not(false)", "true"],
      ["= concat('a', {A2}, '-', {B2})", "a10-Europe"],
      ["= len('hello')", "5"],
      ["= lower('HeLLo')", "hello"],
      ["= upper('abc')", "ABC"],
      ["= trim('  x  ')", "x"],
      ["= lookup('Asia', {B2:B6}, {A2:A6})", "20"],
      ["= match('Europe', {B2:B6})", "1"],
      ["= match('Asia', {B2:B6})", "2"],
      ["= SUM({A2:A3})", "30"],
    ]);
  } finally {
    await app.close();
  }
});

await check("operators, precedence, comparisons and number display", async () => {
  const app = await start();
  try {
    await table(app, [
      ["= 1 + 2 * 3", "7"],
      ["= (1 + 2) * 3", "9"],
      ["= 7 / 2", "3.5"],
      ["= 2 - 5", "-3"],
      ["= -{A2}", "-10"],
      ["= {A2} == 10", "true"],
      ["= {A2} != 10", "false"],
      ["= {A3} >= 20", "true"],
      ["= {B2} == 'Europe'", "true"],
      ["= 0.1 + 0.2", "0.3"],
      ["= 1000000 * 1000000", "1000000000000"],
      ["= {Z99} + 1", "1"],
      ["= {Z99}", ""],
    ]);
  } finally {
    await app.close();
  }
});

await check("numbers as people type them: $1,200 is an amount (not a command), and $, thousands commas and % count in formulas; & joins text", async () => {
  const app = await start();
  try {
    // Typed as a person would, into the grid.
    await cell(app.page, "H2").click();
    await app.page.keyboard.type("$1,200");
    assert.equal(await app.page.locator("#cell-editor").evaluate((e) => e.classList.contains("code")), false, "an amount, not code");
    await cell(app.page, "H5").click();
    const h2 = await get(app, "H2");
    assert.equal(h2.kind, "literal", "not a shell command");
    assert.equal(h2.value, "$1,200");
    assert.equal(await app.page.locator("#cell-editor").evaluate((e) => e.classList.contains("waiting") || e.hidden), true, "the click ended the edit, no {H5} put in");
    await app.call("set_cell", { name: "H3", content: "5%" });
    await app.call("set_cell", { name: "H4", content: "1,000" });
    await table(app, [
      ["= sum({H2:H4})", "2200.05"],
      ["= {H2} * 2", "2400"],
      ["= {H4} * {H3}", "50"],
      ["= countif({H2:H4}, '>999')", "2"],
      ["= {B2} & ' office'", "Europe office"],
      ["= {B2} & ': ' & {A2} + 1", "Europe: 11"],
      ["= {B2} & 'x' == 'Europex'", "true"],
    ]);
  } finally {
    await app.close();
  }
});

await check("a mistake is an error that says what's wrong, not a wrong number", async () => {
  const app = await start();
  try {
    await table(app, [
      ["= 1 / 0", /division by zero/],
      ["= nosuch(1)", /unknown function nosuch/],
      ["= sum()", /expected sum\(range or value/],
      ["= lookup('Nowhere', {B2:B6}, {A2:A6})", /"Nowhere" not found/],
      ["= avg({B2:B6})", /no numeric values/],
      ["= max({B2:B6})", /no numeric values/],
      ["= {B2} * 2", /not a number: "Europe"/],
    ]);
  } finally {
    await app.close();
  }
});

await check("a named cell and a named range read by name, and what reads them follows when they change", async () => {
  const app = await start();
  try {
    await app.call("set_cell", { name: "tax_rate", content: "0.08" });
    await app.call("set_cell", { name: "results", content: "A2:A6" });
    await app.call("set_cell", { name: "D2", content: "= {C2} * (1 + {tax_rate})" });
    await app.call("set_cell", { name: "D3", content: "= sum({results})" });
    assert.equal(await value(app, "D2"), "108.54");
    assert.equal(await value(app, "D3"), "100");
    await app.call("set_cell", { name: "tax_rate", content: "0.1" });
    await app.call("set_cell", { name: "A2", content: "15" });
    await until(async () => (await value(app, "D2")) === "110.55" && (await value(app, "D3")) === "105", { what: "D2 and D3 follow" });
    await app.call("set_cell", { name: "results", content: "A2:A3" });
    await until(async () => (await value(app, "D3")) === "35", { what: "the range moved" });
  } finally {
    await app.close();
  }
});

await check("a cell on another sheet, and a sheet name with a space", async () => {
  const app = await start();
  try {
    await app.call("set_cell", { name: "Notes!A1", content: "5" });
    await app.call("set_cell", { name: "'My Notes'!B2", content: "7" });
    await app.call("set_cell", { name: "D2", content: "= {Notes!A1} * 2" });
    await app.call("set_cell", { name: "D3", content: "= {'My Notes'!B2} + {Notes!A1}" });
    assert.equal(await value(app, "D2"), "10");
    assert.equal(await value(app, "D3"), "12");
    await app.call("set_cell", { name: "Notes!A1", content: "6" });
    await until(async () => (await value(app, "D2")) === "12", { what: "D2 follows the other sheet" });
  } finally {
    await app.close();
  }
});

await check("fill keeps $ anchors and moves the rest; a reference to a deleted row says #REF!; Undo brings it back; a loop is refused", async () => {
  const app = await start();
  try {
    await app.call("set_cell", { name: "D2", content: "= {$A$2} + {A2}" });
    await app.call("fill", { from: "D2", to: "D5", dir: "down" });
    const d4 = await get(app, "D4");
    assert.equal(d4.source, "= {$A$2} + {A4}");
    assert.equal(d4.value, "40");
    await app.call("set_cell", { name: "E2", content: "= {A3} * 2" });
    assert.equal(await value(app, "E2"), "40");
    await app.call("restructure", { axis: "rows", at: 3, n: 1, insert: false });
    await until(async () => /#REF!/.test((await get(app, "E2"))?.source ?? ""), { what: "E2's reference gone", timeout: 5000 });
    await app.menu("edit.undo");
    await until(async () => (await get(app, "E2"))?.source === "= {A3} * 2", { what: "undo restores it" });
    await app.call("set_cell", { name: "G2", content: "= {G3} + 1" });
    // A formula that would loop is refused, saying the loop, and nothing is written.
    const refused = await app.call("set_cell", { name: "G3", content: "= {G2} + 1" }).then(() => "", (e) => e.message);
    assert.match(refused, /cycle: G3 -> G2 -> G3/);
    assert.equal(await value(app, "G3"), "");
  } finally {
    await app.close();
  }
});

await check("a formula built by typing and clicking: = then a click, an operator, a named cell from the list, Enter", async () => {
  const app = await start();
  try {
    await app.call("set_cell", { name: "tax_rate", content: "0.08" });
    await cell(app.page, "D2").click();
    await app.page.keyboard.type("=");
    await cell(app.page, "C2").click();
    await app.page.keyboard.type(" * (1 + {ta");
    const item = app.page.locator("#autocomplete .ac-row", { hasText: "tax_rate" }).first();
    await until(() => item.isVisible(), { what: "tax_rate offered" });
    await app.page.keyboard.press("Enter");
    await app.page.keyboard.type(")");
    assert.equal(await app.page.locator("#cell-editor").inputValue(), "={C2} * (1 + {tax_rate})");
    await app.page.keyboard.press("Enter");
    await until(async () => (await value(app, "D2")) === "108.54", { what: "D2 = 108.54" });
  } finally {
    await app.close();
  }
});

const failed = results.filter(([, r]) => r !== "ok").length;
console.log(`\n${results.length - failed} of ${results.length} passed`);
process.exit(failed ? 1 : 0);
