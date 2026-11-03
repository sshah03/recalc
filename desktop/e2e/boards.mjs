// Making, naming and moving boards: New Board asks nothing and puts each new
// board in the boards folder; the title's name renames or moves it. Free.
// `cargo build -p recalc-desktop`, then `node boards.mjs [filter]`.
import assert from "node:assert/strict";
import { existsSync, mkdirSync, readdirSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { board, open, until } from "./lib.mjs";

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
const dismissWelcome = async (page) => {
  const shown = await page.waitForSelector("#overlay:not([hidden])", { timeout: 4000 }).catch(() => null);
  if (shown) {
    await page.keyboard.press("Escape");
    await page.waitForSelector("#overlay", { state: "hidden", timeout: 5000 });
  }
};
const card = async (app, section, title) => {
  await app.menu("file.new_board");
  await app.page.locator(".start-tab", { hasText: section }).click();
  await app.page.locator(".start-pane:not([hidden]) .card .label", { hasText: new RegExp(`^${title}$`) }).click();
};
const dirNow = async (app) => realpathSync((await app.call("get_board")).dir);
const real = (p) => (existsSync(p) ? realpathSync(p) : p);
// A board of its own is a package on macOS: Name.recalc.
const pkg = (name) => (process.platform === "darwin" ? `${name}.recalc` : name);
const cellValue = async (app, name) => (await app.call("get_board")).cells.find((c) => c.name === name)?.value;
const toast = (app) => app.page.evaluate(() => document.getElementById("toast")?.textContent ?? "");

await check("New Board after an edit asks nothing: the edit stays in its board, and the new board is an empty Untitled board in the boards folder", async () => {
  const dir = board();
  const app = await open(dir);
  try {
    await dismissWelcome(app.page);
    await app.call("set_cell", { name: "A1", content: "keep me" });
    const before = await app.dialogs();
    await card(app, "New", "Blank board");
    const made = join(app.boardsDir, pkg("Untitled board"));
    await until(async () => (await dirNow(app)) === real(made), { what: "the new board" });
    assert.equal(await app.dialogs(), before, "no dialog: nothing was unsaved, and there's nothing to ask");
    assert.equal((await app.call("get_board")).cells.length, 0, "the new board is empty");
    assert.match(readFileSync(join(dir, ".recalc/cells/A1"), "utf8"), /keep me/, "the edit is in the old board's folder");
    assert.equal(await app.page.locator("#board-name").textContent(), "Untitled board");
    assert.match(await toast(app), /New board “Untitled board” in .*recalc-boards-.*\. (Click its name|File → Rename)/, "it says where the board went and how to name it");
    assert.ok(!existsSync(join(dir, pkg("Untitled board"))), "nothing was made inside the old board");
  } finally {
    await app.close();
  }
});

await check("each new board gets a folder of its own: the next free Untitled board, an example under its name, and never a folder that already has files", async () => {
  const dir = board();
  const app = await open(dir);
  try {
    await dismissWelcome(app.page);
    // Someone already has an Untitled board, and an Untitled board 2 with notes in it.
    mkdirSync(join(app.boardsDir, pkg("Untitled board"), ".recalc"), { recursive: true });
    mkdirSync(join(app.boardsDir, pkg("Untitled board 2")), { recursive: true });
    writeFileSync(join(app.boardsDir, pkg("Untitled board 2"), "notes.txt"), "mine");
    await app.call("set_cell", { name: "A1", content: "x" });
    await card(app, "New", "Blank board");
    await until(async () => (await dirNow(app)) === real(join(app.boardsDir, pkg("Untitled board 3"))), { what: "Untitled board 3" });
    assert.equal(readFileSync(join(app.boardsDir, pkg("Untitled board 2"), "notes.txt"), "utf8"), "mine", "an existing folder is left alone");

    // A template card from a board with cells: its own folder, its cells, no welcome over them.
    await app.call("set_cell", { name: "A1", content: "y" });
    await card(app, "Templates", "Verification board");
    const fourth = join(app.boardsDir, pkg("Untitled board 4"));
    await until(async () => (await dirNow(app)) === real(fourth) && (await app.call("get_board")).cells.length > 0, { what: "the template's board" });
    assert.ok(await app.page.locator("#overlay").isHidden(), "no welcome over a board a card made");

    // An example from a board with cells: a package under the example's name, not "recalc-legal".
    await app.call("open_demo", { name: "legal" });
    await until(async () => (await dirNow(app)) === real(join(app.boardsDir, pkg("legal"))), { what: "the example's board" });
    assert.ok((await app.call("get_board")).cells.length > 0);
    assert.equal(await app.dialogs(), 0, "not one dialog in all of that");
  } finally {
    await app.close();
  }
});

await check("a new board left untouched goes when you leave it; one with a cell in it stays", async () => {
  const dir = board();
  const app = await open(dir);
  try {
    await dismissWelcome(app.page);
    await app.call("set_cell", { name: "A1", content: "x" });
    await app.call("new_board");
    const blank = await dirNow(app);
    await app.call("open_board", { path: dir });
    await until(async () => (await dirNow(app)) === real(dir), { what: "back on the first board" });
    assert.ok(!existsSync(blank), "the untouched board's folder is gone");

    await app.call("new_board");
    const kept = await dirNow(app);
    await app.call("set_cell", { name: "B2", content: "worth keeping" });
    await app.call("open_board", { path: dir });
    await until(async () => (await dirNow(app)) === real(dir), { what: "back again" });
    assert.ok(existsSync(join(kept, ".recalc/cells/B2")), "a board with a cell stays");
    // The person's own board is never removed, empty or not.
    await app.call("open_board", { path: kept });
    const empty = board();
    await app.call("open_board", { path: empty });
    await app.call("open_board", { path: dir });
    assert.ok(existsSync(empty), "a board New Board didn't make stays");
  } finally {
    await app.close();
  }
});

await check("a document grid from a board with cells asks only for the documents", async () => {
  const dir = board();
  const app = await open(dir);
  try {
    await dismissWelcome(app.page);
    await app.call("set_cell", { name: "A1", content: "x" });
    const docs = board();
    writeFileSync(join(docs, "a.md"), "a");
    writeFileSync(join(docs, "b.md"), "b");
    await app.pick(docs);
    await card(app, "New", "Document grid from a folder");
    await app.page.waitForSelector("#overlay:not([hidden]) .qcol-question", { timeout: 15000 });
    const b = await app.call("get_board");
    assert.equal(real(b.dir), real(join(app.boardsDir, pkg("Untitled board"))));
    const column = b.cells.filter((c) => /^A\d+$/.test(c.name)).map((c) => c.value.split("/").pop());
    assert.deepEqual(column.sort(), ["Document", "a.md", "b.md"]);
    assert.equal(await app.dialogs(), 1, "one dialog: the documents' folder");
  } finally {
    await app.close();
  }
});

await check("the title's name renames the board where it is; a taken name is said in place; a case-only change works", async () => {
  const home = board();
  const dir = join(home, pkg("Draft"));
  mkdirSync(dir);
  writeFileSync(join(home, "Final"), "a file in the way");
  mkdirSync(join(home, pkg("Final")));
  const app = await open(dir);
  try {
    await dismissWelcome(app.page);
    await app.call("set_cell", { name: "A1", content: "kept" });
    await app.page.click("#board-name");
    const input = app.page.locator("#board-info .bi-name");
    assert.equal(await input.inputValue(), "Draft");
    assert.match(await app.page.locator("#board-info .bi-where").textContent(), /recalc-e2e|tmp|T\//, "it says where the board is");
    await input.fill("Final");
    await input.press("Enter");
    await until(() => app.page.locator("#board-info .bi-said").isVisible(), { what: "the refusal" });
    assert.match(await app.page.locator("#board-info .bi-said").textContent(), /already a “Final”/);
    assert.equal(await dirNow(app), real(dir), "the board stays put");

    await input.fill("Q3 review");
    await input.press("Enter");
    const moved = join(home, pkg("Q3 review"));
    await until(async () => (await dirNow(app)) === real(moved), { what: "the renamed board" });
    assert.ok(!existsSync(dir), "the old name is gone");
    assert.equal(await cellValue(app, "A1"), "kept", "its cells came with it");
    assert.ok(await app.page.locator("#board-info").isHidden(), "the popover closes");
    assert.equal(await app.page.locator("#board-name").textContent(), "Q3 review");
    await app.call("set_cell", { name: "A2", content: "after" });
    await until(() => existsSync(join(moved, ".recalc/cells/A2")), { what: "a write after the rename lands in the new folder" });

    await app.call("rename_board", { name: "q3 Review" });
    await until(async () => readdirSync(home).includes(pkg("q3 Review")), { what: "the case-only rename" });
    assert.equal(await cellValue(app, "A1"), "kept");
    const bad = await app.call("rename_board", { name: "a/b" }).then(() => "", (e) => e.message);
    assert.match(bad, /can't .* contain \//);
  } finally {
    await app.close();
  }
});

await check("Move To… puts the board in a picked folder and works there; it's refused while a cell runs and into its own folder", async () => {
  const home = board();
  const dir = join(home, pkg("Ledger"));
  mkdirSync(dir);
  const app = await open(dir);
  try {
    await dismissWelcome(app.page);
    await app.call("set_cell", { name: "A1", content: "1" });
    await app.call("set_cell", { name: "B1", content: "$ sleep 30" });
    await app.call("trigger", { name: "B1" }).catch(() => {});
    const elsewhere = board();
    await app.pick(elsewhere);
    const busy = await app.call("move_board").then(() => "", (e) => e.message);
    assert.match(busy, /running/, "refused while a cell runs");
    assert.equal(await dirNow(app), real(dir));
    await app.call("stop", { name: "B1" });
    await until(async () => !["computing", "queued"].includes((await app.call("get_board")).cells.find((c) => c.name === "B1")?.state), { what: "B1 stopped" });

    await app.pick(join(dir, "sub"));
    mkdirSync(join(dir, "sub"));
    const inside = await app.call("move_board").then(() => "", (e) => e.message);
    assert.match(inside, /into its own folder/);

    await app.pick(elsewhere);
    const to = await app.call("move_board");
    assert.equal(real(to), real(join(elsewhere, pkg("Ledger"))));
    assert.ok(!existsSync(dir));
    await app.call("set_cell", { name: "C1", content: "= {A1} + 1" });
    await until(async () => (await cellValue(app, "C1")) === "2", { what: "the moved board computes" });
  } finally {
    await app.close();
  }
});

const failed = results.filter(([, r]) => r !== "ok").length;
console.log(`\n${results.length - failed} of ${results.length} passed`);
process.exit(failed ? 1 : 0);
