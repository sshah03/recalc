// Walks the column filter and a waiting session, taking screenshots.
// Free (stand-in agent). `node walk-sessions.mjs`.
import { writeFileSync, mkdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { board, open, cell, until } from "./lib.mjs";
const here = dirname(fileURLToPath(import.meta.url));
const note = (s) => console.log("•", s);
const cellOf = async (app, n) => (await app.call("get_board")).cells.find((c) => c.name === n);

// ---- 1. Column filter on the practice demo ----
{
  const app = await open(board("practice"));
  try {
    await app.page.waitForTimeout(1500);
    const b = await app.call("get_board");
    const headers = b.cells.filter((c) => /^[A-Z]1$/.test(c.name)).map((c) => `${c.name}=${c.value.slice(0, 18)}`);
    note(`practice headers: ${headers.join(" | ")}`);
    // The agents haven't run, so fill Status by hand to have something to filter.
    const statuses = ["Ready to file", "Waiting on client", "In progress", "Waiting on client", "Ready to file", "In progress", "Waiting on client", "Ready to file"];
    for (let i = 0; i < 8; i++) await app.call("set_cell", { name: `C${i + 2}`, content: statuses[i] });
    await app.page.waitForTimeout(400);
    await cell(app.page, "C3").click();
    await app.page.waitForTimeout(200);
    note(`selected: ${await app.page.textContent("#namebox")}`);
    await app.menu("view.filter_column");
    await app.page.waitForSelector("#overlay:not([hidden]) .value-list", { timeout: 5000 });
    await app.shot("walk-filter-picker");
    note(`picker: ${(await app.page.textContent("#overlay")).replace(/\s+/g, " ").slice(0, 160)}`);
    // Keep only "Waiting on client".
    for (const label of ["In progress", "Ready to file"]) await app.page.locator(".value-row", { hasText: label }).locator("input").uncheck();
    await app.page.locator("#overlay button", { hasText: "Show these rows" }).click();
    await app.page.waitForTimeout(400);
    note(`status bar: ${await app.page.textContent("#status-filter")}`);
    const shown = await app.page.evaluate(() => [...new Set([...document.querySelectorAll("#cells .cell, #frozen-cols .cell")].map((e) => e.dataset.name.replace(/^[A-Z]+/, "")))].sort((a, b) => a - b));
    note(`rows showing: ${shown.join(",")}`);
    await app.shot("walk-filter-applied");
    await app.page.click("#status-filter");
    await app.page.waitForTimeout(300);
    note(`after clearing: "${await app.page.textContent("#status-filter")}"`);
  } finally {
    await app.close();
  }
}

// ---- 2. A session that finishes and waits at its prompt ----
{
  const dir = board();
  mkdirSync(join(dir, ".recalc"), { recursive: true });
  writeFileSync(join(dir, ".recalc/config.toml"), `[paths]\nclaude = "${join(here, "fake-agent.py")}"\n`);
  const app = await open(dir);
  try {
    await app.page.keyboard.press("Escape");
    await app.call("set_cell", { name: "B2", content: "@claude('fix the report')" });
    await app.call("session_open", { name: "B2", cols: 100, rows: 30 });
    await until(async () => (await cellOf(app, "B2_status"))?.value === "waiting?", { timeout: 20000, what: "B2_status to read waiting?" });
    note(`B2_status: ${(await cellOf(app, "B2_status")).value}`);
    await app.page.click("#tab-decisions");
    await until(async () => (await app.page.textContent("#tray")).includes("at the prompt"), { timeout: 5000, what: "the waiting row" });
    await app.shot("walk-session-waiting");
    note(`Needs You: ${(await app.page.textContent("#tray")).replace(/\s+/g, " ").slice(0, 200)}`);
    note(`count: ${await app.page.textContent("#needs-count")}`);
    await app.page.locator("#tray .need-waiting button", { hasText: "Show" }).click();
    await app.page.waitForTimeout(800);
    note(`after Show, selected: ${await app.page.textContent("#namebox")}; inspector: ${(await app.page.textContent("#inspector")).replace(/\s+/g, " ").slice(0, 120)}`);
    await app.shot("walk-session-shown");
    await app.page.click("#tab-decisions");
    await app.page.locator("#tray .need-waiting button", { hasText: "End session" }).click();
    await until(async () => !/^(running|waiting)/.test((await cellOf(app, "B2_status"))?.value ?? ""), { timeout: 10000, what: "the session to end" });
    await app.page.waitForTimeout(500);
    note(`after End session: B2_status=${(await cellOf(app, "B2_status")).value}; row gone: ${!(await app.page.textContent("#tray")).includes("at the prompt")}`);
    note(`page errors: ${JSON.stringify(app.errors)}`);
  } finally {
    await app.close();
  }
}
