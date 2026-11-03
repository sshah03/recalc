// Starts the driver on a fresh folder, answers Claude Code's trust prompt,
// asks for a table and waits for it. Paid (about $0.50). `npm run driver`.
import { rmSync } from "node:fs";
import { board, open, until } from "./lib.mjs";

const note = (s) => console.log("•", s);
const dir = board("legal");
rmSync(dir + "/.recalc", { recursive: true, force: true });
rmSync(dir + "/board.recalctemplate", { force: true });
const app = await open(dir);
const { page } = app;
if (await page.isVisible("#overlay")) await page.keyboard.press("Escape");
await page.click("#tab-driver");
await page.locator("#tray button", { hasText: "Start" }).click();
await until(async () => (await page.locator("#tray .xterm").count()) > 0, { timeout: 30000, what: "the driver's terminal" });
const banner = await until(async () => (await page.isVisible("#tray .term-banner")) || null, { timeout: 30000, what: "the trust banner" }).catch(() => false);
note(`trust banner shown: ${!!banner}`);
await app.shot("driver-trust");
if (banner) {
  await page.locator("#tray .xterm").click();
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  await until(async () => !(await page.isVisible("#tray .term-banner")), { timeout: 30000, what: "the banner to go once answered" }).catch((e) => note(String(e)));
}
await page.waitForTimeout(5000);
await app.shot("driver-ready");
await page.locator("#tray .xterm").click();
await page.keyboard.type("Build a document grid over the contracts in contracts/: counterparty and governing law. Run the first row only.", { delay: 15 });
await page.keyboard.press("Enter");
const t0 = Date.now();
await until(async () => (await app.call("get_board")).cells.some((c) => c.name === "A2"), { timeout: 300000, every: 3000, what: "the grid" });
note(`grid appeared after ${Math.round((Date.now() - t0) / 1000)}s`);
await page.waitForTimeout(2000);
await app.shot("driver-grid");
await page.click("#tab-decisions");
await until(async () => {
  const b = await app.call("get_board");
  const row = b.cells.filter((c) => /^[B-Z]2$/.test(c.name) && c.kind === "agent_call");
  return row.length > 0 && row.every((c) => c.value && c.state !== "computing" && c.state !== "queued");
}, { timeout: 400000, every: 3000, what: "the first row" }).catch((e) => note(String(e)));
note(`row answered after ${Math.round((Date.now() - t0) / 1000)}s`);
await app.shot("driver-row");
const b = await app.call("get_board");
const byName = (n) => b.cells.find((c) => c.name === n);
note(`headers: ${b.cells.filter((c) => /^[A-Z]1$/.test(c.name)).map((c) => `${c.name}=${c.value}`).join(" | ")}`);
note(`row 2: ${b.cells.filter((c) => /^[A-Z]2$/.test(c.name)).map((c) => `${c.name}=${String(c.value).slice(0, 40)}${c.cite ? ` [cite found=${c.cite.found}]` : ""}`).join(" | ")}`);
note(`shapes on answers: ${b.cells.filter((c) => c.answer_type && /^[B-Z]\d+$/.test(c.name) && !/1$/.test(c.name)).length}`);
note(`named cells: ${b.cells.filter((c) => !/^[A-Z]+\d+$/.test(c.name)).map((c) => c.name).join(", ")}`);
note(`B2 source: ${byName("B2")?.source}`);
note(`page errors: ${JSON.stringify(app.errors)}`);
await app.close();
