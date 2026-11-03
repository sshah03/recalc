// Named cells: made from the side panel, edited in their own row, and said
// how to use. Free. `cargo build -p recalc-desktop`, then `node named.mjs [filter]`.
import assert from "node:assert/strict";
import { board, cell, open, until } from "./lib.mjs";

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
const start = async () => {
  const app = await open(board());
  const shown = await app.page.waitForSelector("#overlay:not([hidden])", { timeout: 4000 }).catch(() => null);
  if (shown) {
    await app.page.keyboard.press("Escape");
    await app.page.waitForSelector("#overlay", { state: "hidden" });
  }
  return app;
};
const editorIn = (app) => app.page.evaluate(() => document.getElementById("cell-editor")?.closest(".named-row")?.dataset.name ?? null);
const scrollTop = (app) => app.page.evaluate(() => document.getElementById("grid-wrap").scrollTop);
const value = async (app, name) => (await app.call("get_board")).cells.find((c) => c.name === name)?.value;
async function newCell(app, name) {
  await app.page.locator(".named-row.add").click();
  await app.page.keyboard.type(name);
  await app.page.keyboard.press("Enter");
}

await check("a new named cell is typed into its own row, and the grid stays where it was", async () => {
  const app = await start();
  try {
    assert.match(await app.page.locator("#named .named-help").textContent(), /instead of a place.*\{budget\}/, "the empty list says what they are");
    // Typed straight after the name, before the cell's row has appeared.
    await newCell(app, "budget");
    await app.page.keyboard.type("50");
    await until(async () => (await editorIn(app)) === "budget", { what: "the editor in budget's row" });
    assert.ok(await app.page.locator("#cell-editor").isVisible());
    assert.equal(await app.page.evaluate(() => document.activeElement?.id), "cell-editor", "the typing moved to the row");
    assert.equal(await scrollTop(app), 0, "the grid didn't jump");
    await app.page.keyboard.type("0");
    assert.equal(await app.page.locator("#cell-editor").inputValue(), "500", "nothing typed was lost");
    await app.page.keyboard.press("Enter");
    await until(async () => (await value(app, "budget")) === "500", { what: "budget = 500" });
    assert.equal(await editorIn(app), null, "the editor went back to the grid");
    assert.equal(await app.page.locator("#named .named-help").count(), 0, "the explainer is for an empty list");
    assert.match(await app.page.locator("#named .named-row[data-name=budget] .face").textContent(), /500/);
    await app.shot("named-made");
  } finally {
    await app.close();
  }
});

await check("an edit in a named row survives the board changing underneath it, and a click inside it keeps it", async () => {
  const app = await start();
  try {
    await app.call("set_cell", { name: "rate", content: "1" });
    await app.page.locator("#named .named-row[data-name=rate]").dblclick();
    await until(async () => (await editorIn(app)) === "rate", { what: "editing rate" });
    await app.page.keyboard.press("Meta+a");
    await app.page.keyboard.type("0.2");
    // Something else changes the board: the list redraws.
    await app.call("set_cell", { name: "A1", content: "elsewhere" });
    await app.call("set_cell", { name: "zeta", content: "new" });
    await until(async () => (await app.page.locator("#named .named-row[data-name=zeta]").count()) === 1, { what: "the list redrawn" });
    assert.equal(await editorIn(app), "rate", "still editing in its row");
    assert.equal(await app.page.evaluate(() => document.activeElement?.id), "cell-editor", "focus kept");
    await app.page.locator("#cell-editor").click();
    assert.equal(await editorIn(app), "rate", "a click inside places the caret, nothing more");
    await app.page.keyboard.type("5");
    await app.page.keyboard.press("Enter");
    await until(async () => (await value(app, "rate")) === "0.25", { what: "rate = 0.25" });
    const order = await app.page.locator("#named .named-row:not(.add)").evaluateAll((rows) => rows.map((r) => r.dataset.name));
    assert.deepEqual(order, ["rate", "zeta"], "rows in order after the redraws");
  } finally {
    await app.close();
  }
});

await check("the Inspector says how to use a named cell and what uses it; {bu offers it while typing", async () => {
  const app = await start();
  try {
    await app.call("set_cell", { name: "budget", content: "500" });
    await app.page.locator("#named .named-row[data-name=budget]").click();
    const use = app.page.locator("#inspector-body .use");
    await until(() => use.isVisible(), { what: "the use section" });
    assert.match(await use.textContent(), /Other cells use it as \{budget\}/);
    assert.match(await use.textContent(), /Nothing uses it yet/);
    await cell(app.page, "B1").click();
    await app.page.keyboard.type("= {bu");
    await until(() => app.page.locator("#autocomplete .ac-row", { hasText: "budget" }).first().isVisible(), { what: "budget offered" });
    await app.page.keyboard.press("Escape");
    await app.page.keyboard.press("Escape");
    await app.call("set_cell", { name: "B1", content: "= {budget} * 2" });
    await app.page.locator("#named .named-row[data-name=budget]").click();
    await until(async () => /Used by\s*B1/.test(await use.textContent()), { what: "used by B1" });
    await app.shot("named-used-by");
    await use.locator(".act-cell", { hasText: "B1" }).click();
    await until(async () => (await app.page.textContent("#namebox")) === "B1", { what: "B1 selected" });
  } finally {
    await app.close();
  }
});

await check("with the list folded, a named cell is edited in the formula bar, and the grid stays put", async () => {
  const app = await start();
  try {
    await app.call("set_cell", { name: "note", content: "hi" });
    await app.page.locator("#named .named-row[data-name=note]").click();
    await app.page.click("#named-head");
    assert.equal(await app.page.locator("#named .named-row").count(), 0, "folded");
    await app.page.keyboard.press("Enter");
    await until(async () => (await app.page.evaluate(() => document.activeElement?.id)) === "formula", { what: "the formula bar editing" });
    assert.equal(await scrollTop(app), 0);
    await app.page.keyboard.press("Meta+a");
    await app.page.keyboard.type("hello");
    await app.page.keyboard.press("Enter");
    await until(async () => (await value(app, "note")) === "hello", { what: "note = hello" });
  } finally {
    await app.close();
  }
});

const failed = results.filter(([, r]) => r !== "ok").length;
console.log(`\n${results.length - failed} of ${results.length} passed`);
process.exit(failed ? 1 : 0);
