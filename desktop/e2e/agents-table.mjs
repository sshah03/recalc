// Real agents through the UI: runs, reads and verifies a legal-demo row, then
// a headless Claude cell that must ask before using Bash. Paid (about $1).
// `npm run agents`.
import { board, open, cell, until } from "./lib.mjs";

const note = (s) => console.log("•", s);
const valueOf = async (app, n) => (await app.call("get_board")).cells.find((c) => c.name === n);

// ---- 1. Legal demo: run, read and verify one row ----
{
  const app = await open(board("legal"));
  const { page } = app;
  await cell(page, "B2").click();
  await cell(page, "F2").click({ modifiers: ["Shift"] });
  await app.menu("cell.run_selection");
  await page.click("#tab-decisions");
  await page.waitForTimeout(4000);
  await app.shot("legal-running");
  note(`while running, the tray says: ${(await page.textContent("#tray")).replace(/\s+/g, " ").slice(0, 240)}`);
  await until(async () => {
    const b = await app.call("get_board");
    return ["B2", "C2", "D2", "E2", "F2"].every((n) => {
      const c = b.cells.find((x) => x.name === n);
      return c && c.state !== "computing" && c.state !== "queued";
    });
  }, { timeout: 300000, every: 2000, what: "row 2 to finish" });
  const b = await app.call("get_board");
  for (const n of ["B2", "C2", "D2", "E2", "F2"]) {
    const c = b.cells.find((x) => x.name === n);
    note(`${n} = ${JSON.stringify(c.value).slice(0, 70)} state=${JSON.stringify(c.state)} cite=${c.cite ? `${c.cite.file} found=${c.cite.found}` : "none"} cost=${c.usage?.cost_usd?.toFixed(3)}`);
  }
  await app.shot("legal-row-done");
  await cell(page, "D2").click();
  await page.waitForTimeout(300);
  const show = page.locator("#inspector-body button", { hasText: "Show source" });
  if (await show.count()) {
    await show.first().click();
    await page.waitForTimeout(800);
    note(`source pane: ${(await page.textContent("#tray")).replace(/\s+/g, " ").slice(0, 160)}`);
    note(`highlighted passage shown: ${await page.locator("#tray mark").count()}`);
  } else note("no Show source button on D2");
  await app.shot("legal-source");
  await app.menu("cell.verify");
  await page.waitForTimeout(300);
  note(`D2 verified: ${(await valueOf(app, "D2")).mark?.kind}`);
  note(`status bar: ${(await page.textContent("#statusbar")).replace(/\s+/g, " ")}`);
  note(`page errors: ${JSON.stringify(app.errors)}`);
  await app.close();
}

// ---- 2. Headless Claude cell asking for a tool it wasn't given ----
{
  const app = await open(board());
  const { page } = app;
  if (await page.isVisible("#overlay")) await page.keyboard.press("Escape");
  await cell(page, "A1").click();
  await page.keyboard.type("@claude('Create a directory named e2e-made here with the Bash tool (mkdir e2e-made), then answer with the single word done.')", { delay: 5 });
  await page.keyboard.press("Enter");
  await cell(page, "A1").click();
  await app.menu("cell.run");
  await page.click("#tab-decisions");
  await until(async () => (await page.textContent("#tray")).includes("wants to use"), { timeout: 120000, what: "the permission question" });
  await app.shot("permission-question");
  note(`question row: ${(await page.textContent("#tray")).replace(/\s+/g, " ").slice(0, 200)}`);
  await page.locator("#tray .need-ask .need-acts button").first().click();
  await until(async () => (await valueOf(app, "A1"))?.value === "done", { timeout: 120000, what: "the run to finish after Allow" });
  const { existsSync } = await import("node:fs");
  note(`A1 = done; the directory exists: ${existsSync(app.dir + "/e2e-made")}`);
  await app.shot("permission-allowed");
  note(`page errors: ${JSON.stringify(app.errors)}`);
  await app.close();
}
