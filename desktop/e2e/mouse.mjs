// The mouse on the grid, as in Sheets and Excel: clicking away from an edit,
// picking references into a formula, the fill handle, and selecting by drag.
// Free. `cargo build -p recalc-desktop`, then `node mouse.mjs [filter]`.
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
    console.log(`FAIL ${name}\n     ${String(e.stack ?? e.message ?? e).split("\n").slice(0, 6).join("\n     ")}`);
  }
}

/** A board with numbers in B2:B4 and a run of names in A2:A6. */
async function start() {
  const app = await open(board());
  const shown = await app.page.waitForSelector("#overlay:not([hidden])", { timeout: 4000 }).catch(() => null);
  if (shown) {
    await app.page.keyboard.press("Escape");
    await app.page.waitForSelector("#overlay", { state: "hidden" });
  }
  for (const [n, v] of [["B2", "2"], ["B3", "3"], ["B4", "4"], ["A2", "a"], ["A3", "b"], ["A4", "c"], ["A5", "d"], ["A6", "e"]]) {
    await app.call("set_cell", { name: n, content: v });
  }
  return app;
}
const p = (app) => app.page;
const ed = (app) => app.page.locator("#cell-editor");
const editing = (app) => app.page.evaluate(() => document.body.classList.contains("editing"));
const namebox = (app) => app.page.textContent("#namebox");
const get = async (app, n) => (await app.call("get_board")).cells.find((c) => c.name === n);
const value = async (app, n) => (await get(app, n))?.value ?? "";
const source = async (app, n) => {
  const c = await get(app, n);
  return c?.source ?? c?.value ?? "";
};
const center = async (app, n) => {
  const b = await cell(app.page, n).boundingBox();
  return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
};
async function drag(app, from, to) {
  const a = await center(app, from);
  const b = await center(app, to);
  await app.page.mouse.move(a.x, a.y);
  await app.page.mouse.down();
  await app.page.mouse.move((a.x + b.x) / 2, (a.y + b.y) / 2, { steps: 4 });
  await app.page.mouse.move(b.x, b.y, { steps: 4 });
  await app.page.mouse.up();
}
/** Select a cell and type into it, as a person starting an entry. */
async function typeInto(app, n, text) {
  await cell(app.page, n).click();
  await app.page.keyboard.type(text);
  await until(() => editing(app), { what: `editing ${n}`, timeout: 3000 });
}

await check("double-click a value, then click another cell: the edit is saved as typed, no reference goes in, the other cell is selected", async () => {
  const app = await start();
  try {
    await app.call("set_cell", { name: "C2", content: "Acme" });
    await cell(p(app), "C2").dblclick();
    await until(() => editing(app), { what: "editing C2" });
    await p(app).keyboard.type(" Inc");
    await cell(p(app), "D5").click();
    await until(async () => (await value(app, "C2")) === "Acme Inc", { what: "C2 saved" });
    assert.equal(await namebox(app), "D5");
    assert.equal(await editing(app), false, "no cursor left in C2");
    assert.equal(await value(app, "D5"), "", "nothing went into D5");
  } finally {
    await app.close();
  }
});

await check("double-click an empty cell, then click another: no reference goes in, and the other cell is selected", async () => {
  const app = await start();
  try {
    await cell(p(app), "C3").dblclick();
    await until(() => editing(app), { what: "editing C3" });
    assert.equal(await ed(app).inputValue(), "");
    await cell(p(app), "B3").click();
    await until(async () => (await namebox(app)) === "B3", { what: "B3 selected" });
    assert.equal(await editing(app), false);
    assert.equal(await source(app, "C3"), "", "C3 stays empty, not {B3}");
    assert.equal(await value(app, "B3"), "3");
  } finally {
    await app.close();
  }
});

await check("a formula takes references from clicks: a click puts one in, another click replaces it, after + a click adds one", async () => {
  const app = await start();
  try {
    await typeInto(app, "D2", "=");
    await cell(p(app), "B2").click();
    assert.equal(await ed(app).inputValue(), "={B2}");
    await cell(p(app), "B3").click();
    assert.equal(await ed(app).inputValue(), "={B3}", "a second click replaces the reference just picked");
    await p(app).keyboard.type("+");
    await cell(p(app), "B4").click();
    assert.equal(await ed(app).inputValue(), "={B3}+{B4}");
    await p(app).keyboard.press("Enter");
    await until(async () => (await value(app, "D2")) === "7", { what: "D2 = 7" });
  } finally {
    await app.close();
  }
});

await check("a drag while picking puts a range in", async () => {
  const app = await start();
  try {
    await typeInto(app, "D3", "= sum(");
    await drag(app, "B2", "B4");
    assert.equal(await ed(app).inputValue(), "= sum({B2:B4}");
    await p(app).keyboard.type(")");
    await p(app).keyboard.press("Enter");
    await until(async () => (await value(app, "D3")) === "9", { what: "D3 = 9" });
  } finally {
    await app.close();
  }
});

await check("a click after a finished formula saves it and selects the cell clicked", async () => {
  const app = await start();
  try {
    await typeInto(app, "D4", "= 1 + 2");
    await cell(p(app), "B2").click();
    await until(async () => (await value(app, "D4")) === "3", { what: "D4 saved" });
    assert.equal(await namebox(app), "B2");
    assert.equal(await editing(app), false);
  } finally {
    await app.close();
  }
});

await check("an agent's prompt and a command take references from clicks; Escape drops the whole edit", async () => {
  const app = await start();
  try {
    await typeInto(app, "E2", "@claude('summarize ");
    await cell(p(app), "B2").click();
    assert.equal(await ed(app).inputValue(), "@claude('summarize {B2}");
    await p(app).keyboard.press("Escape");
    assert.equal(await editing(app), false);
    assert.equal(await source(app, "E2"), "", "Escape left E2 as it was");

    await typeInto(app, "E3", "$ echo ");
    await cell(p(app), "B3").click();
    assert.equal(await ed(app).inputValue(), "$ echo {B3}");
    await p(app).keyboard.press("Enter");
    await until(async () => (await value(app, "E3")) === "3", { what: "E3 echoes 3" });
  } finally {
    await app.close();
  }
});

await check("the formula bar takes picked references too", async () => {
  const app = await start();
  try {
    await cell(p(app), "G2").click();
    await p(app).click("#formula");
    await p(app).keyboard.type("=");
    assert.ok(await p(app).locator("#autocomplete").isHidden(), "a bare = opens no list over the grid");
    await cell(p(app), "B4").click();
    assert.equal(await p(app).locator("#formula").inputValue(), "={B4}");
    await p(app).keyboard.press("Enter");
    await until(async () => (await value(app, "G2")) === "4", { what: "G2 = 4" });
    // A name in progress still offers functions.
    await typeInto(app, "G3", "=su");
    await until(() => p(app).locator("#autocomplete .ac-row", { hasText: "sum(" }).first().isVisible(), { what: "sum offered" });
    await p(app).keyboard.press("Escape");
  } finally {
    await app.close();
  }
});

await check("double-click another cell while editing: the first is saved and the second edited; a click inside the editor keeps editing", async () => {
  const app = await start();
  try {
    await app.call("set_cell", { name: "F2", content: "one" });
    await app.call("set_cell", { name: "F3", content: "two" });
    await cell(p(app), "F2").dblclick();
    await p(app).keyboard.type(" more");
    await cell(p(app), "F3").dblclick();
    await until(async () => (await value(app, "F2")) === "one more", { what: "F2 saved" });
    await until(async () => (await ed(app).inputValue()) === "two", { what: "editing F3" });
    const box = await ed(app).boundingBox();
    await p(app).mouse.click(box.x + 8, box.y + box.height / 2);
    assert.equal(await editing(app), true, "still editing F3");
    assert.equal(await ed(app).inputValue(), "two");
    await p(app).keyboard.press("Escape");
  } finally {
    await app.close();
  }
});

await check("the fill handle: a drag copies a formula down with relative references, a double click fills down beside a column", async () => {
  const app = await start();
  try {
    await app.call("set_cell", { name: "C2", content: "= {B2} * 10" });
    await cell(p(app), "C2").click();
    const h = await p(app).locator("#fill-handle").boundingBox();
    const to = await center(app, "C4");
    await p(app).mouse.move(h.x + h.width / 2, h.y + h.height / 2);
    await p(app).mouse.down();
    await p(app).mouse.move(to.x, to.y, { steps: 6 });
    await p(app).mouse.up();
    await until(async () => (await value(app, "C4")) === "40", { what: "C4 = 40" });
    assert.match(await source(app, "C3"), /\{B3\} \* 10/);
    assert.equal(await value(app, "C5"), "", "only as far as the drag");

    // A run of names in A2:A6: a double click on B's handle fills B to row 6.
    await app.call("set_cell", { name: "D2", content: "= upper({A2})" });
    await cell(p(app), "D2").click();
    await app.call("set_cell", { name: "C5", content: "x" });
    await app.call("set_cell", { name: "C6", content: "y" });
    const h2 = await p(app).locator("#fill-handle").boundingBox();
    await p(app).mouse.dblclick(h2.x + h2.width / 2, h2.y + h2.height / 2);
    await until(async () => (await value(app, "D6")) === "E", { what: "D6 = E" });
    assert.equal(await value(app, "D7"), "", "no further than the column beside");
  } finally {
    await app.close();
  }
});

await check("dragging across cells selects a block, Shift-click extends it, and a header click selects its column or row", async () => {
  const app = await start();
  try {
    await drag(app, "B2", "C4");
    assert.equal(await namebox(app), "B2:C4");
    await cell(p(app), "B2").click();
    await cell(p(app), "D5").click({ modifiers: ["Shift"] });
    assert.equal(await namebox(app), "B2:D5");
    await p(app).locator(".hdr.col[data-col='2']").click();
    assert.match(await namebox(app), /^B1:B\d+$/);
    await p(app).locator(".hdr.row[data-row='3']").click();
    assert.match(await namebox(app), /^A3:[A-Z]+3$/);
    assert.equal(await editing(app), false, "selecting never starts an edit");
  } finally {
    await app.close();
  }
});

const failed = results.filter(([, r]) => r !== "ok").length;
console.log(`\n${results.length - failed} of ${results.length} passed`);
process.exit(failed ? 1 : 0);
