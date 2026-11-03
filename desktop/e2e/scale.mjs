// Times common edits on a big table: paste, fill, edit, scroll, sort, undo,
// delete rows, a long formula chain. Free (no agents).
//   node scale.mjs [rows]                      the debug build
//   RECALC_BIN=…/target/perf/release/recalc node scale.mjs 1000
// A release build that still has the bridge:
//   CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true cargo build -p recalc-desktop --release --target-dir target/perf
// Steps that wait on the backend include one get_board poll.
import { board, open, cell, until } from "./lib.mjs";
const N = Number(process.argv[2] ?? 1000);
const app = await open(board());
const get = async (n) => (await app.call("get_board")).cells.find((c) => c.name === n);
const time = async (label, f) => { const t = Date.now(); await f(); console.log(`${label}: ${Date.now() - t} ms`); };
await app.page.keyboard.press("Escape").catch(() => {});
const rows = [["doc", "a", "b", "note"]];
for (let i = 1; i <= N; i++) rows.push([`docs/file-${i}.md`, String(i), String(i * 2), `note ${i} lorem ipsum dolor`]);
await time(`paste ${N}x4`, () => app.call("paste_text", { at: "A1", text: rows.map((r) => r.join("\t")).join("\n") }));
await time(`fill formula ${N} rows`, async () => {
  await app.call("set_cell", { name: "E2", content: "= {B2} + {C2}" });
  await app.call("fill", { from: "E2", to: `E${N + 1}`, dir: "down" });
  await until(async () => (await get(`E${N + 1}`))?.value === String(N * 3), { timeout: 120000, what: "last formula" });
});
await time("summary over the column", async () => {
  await app.call("set_cell", { name: "total", content: `= sum({E2:E${N + 1}})` });
  await until(async () => (await get("total"))?.value === String(3 * N * (N + 1) / 2), { timeout: 60000, what: "total" });
});
await time("one input edit ripples (B2 → E2 → total)", async () => {
  await app.call("set_cell", { name: "B2", content: "100" });
  await until(async () => (await get("E2"))?.value === "102" , { timeout: 60000, what: "ripple" });
});
await time("get_board", () => app.call("get_board"));
await time("typed edit drawn", async () => {
  await cell(app.page, "F3").click(); await app.page.keyboard.type("x"); await app.page.keyboard.press("Enter");
  await until(async () => (await app.page.textContent(`#cells .cell[data-name='F3']`))?.includes("x"), { timeout: 30000, what: "typed" });
});
await time("scroll to bottom", async () => { await app.page.mouse.move(600, 400); for (let i = 0; i < 20; i++) await app.page.mouse.wheel(0, 3000); await app.page.waitForTimeout(100); });
await time("sort descending", async () => {
  await app.call("sort_rows", { col: 2, first: 2, last: N + 1, descending: true });
  await until(async () => (await get("A2"))?.value === `docs/file-${N}.md`, { timeout: 120000, what: "sort" });
});
await time("undo the sort", async () => {
  await app.menu("edit.undo");
  await until(async () => (await get("A2"))?.value === "docs/file-1.md", { timeout: 120000, what: "undo" });
});
await time("delete 100 rows", async () => {
  await app.call("restructure", { axis: "rows", at: 2, n: 100, insert: false }).catch(async (e) => { console.log("  delete_rows:", e.message); });
  await until(async () => (await get("A2"))?.value === "docs/file-101.md", { timeout: 120000, what: "delete" });
});
await time("chain of 300 formulas", async () => {
  await app.call("set_cell", { name: "c0", content: "1" });
  const cells = {}; for (let i = 1; i <= 300; i++) cells[`c${i}`] = `= {c${i - 1}} + 1`;
  for (const [k, v] of Object.entries(cells)) await app.call("set_cell", { name: k, content: v });
  await until(async () => (await get("c300"))?.value === "301", { timeout: 120000, what: "chain" });
});
await time("edit the chain's head", async () => {
  await app.call("set_cell", { name: "c0", content: "1000" });
  await until(async () => (await get("c300"))?.value === "1300", { timeout: 120000, what: "chain ripple" });
});
console.log("cells:", (await app.call("get_board")).cells.length, "errors:", app.errors.slice(0, 3));
await app.close();
