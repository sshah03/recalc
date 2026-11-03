// Six agent cells, a CLI loop, a second MCP client and a person typing all
// write `shared` (and their own cells) at once; then window, engine and disk
// must agree with nothing torn, lost or stuck. Free (stand-in agents).
//   node concurrency.mjs
import assert from "node:assert/strict";
import { spawn, execFile } from "node:child_process";
import { promisify } from "node:util";
import { readFileSync, writeFileSync, mkdirSync, existsSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { board, open, cell, until, recalc } from "./lib.mjs";

const run = promisify(execFile);
const here = dirname(fileURLToPath(import.meta.url));
const dir = board();
mkdirSync(join(dir, ".recalc"), { recursive: true });
writeFileSync(join(dir, ".recalc/config.toml"), `max_agents = 6\n[paths]\nclaude = "${join(here, "fake-agent.py")}"\n`);
const app = await open(dir);
const t0 = Date.now();
try {
  await app.page.keyboard.press("Escape").catch(() => {});
  const agents = ["B2", "B3", "B4", "B5", "B6", "B7"];
  for (const n of agents) await app.call("set_cell", { name: n, content: `@claude('fake: set=shared|from-${n}; set=own_${n}|done; loop=25; answer=ok-${n}')` });
  await app.call("run_selection", { names: agents });

  const cli = (async () => {
    for (let i = 0; i < 25; i++) await run(recalc, ["set", "shared", `from-cli-${i}`], { cwd: dir });
    await run(recalc, ["set", "cli_done", "yes"], { cwd: dir });
  })();

  // A second MCP client, as an interactive agent would be.
  const mcp = (async () => {
    const p = spawn(recalc, ["mcp"], { cwd: dir, stdio: ["pipe", "pipe", "ignore"] });
    let buf = "";
    const replies = new Map();
    p.stdout.on("data", (d) => {
      buf += d;
      for (let i; (i = buf.indexOf("\n")) >= 0; ) {
        const line = buf.slice(0, i);
        buf = buf.slice(i + 1);
        try { const m = JSON.parse(line); if (m.id != null) replies.get(m.id)?.(m); } catch {}
      }
    });
    let id = 0;
    const rpc = (method, params) => new Promise((res) => { const n = ++id; replies.set(n, res); p.stdin.write(JSON.stringify({ jsonrpc: "2.0", id: n, method, params }) + "\n"); });
    await rpc("initialize", { protocolVersion: "2025-06-18", clientInfo: { name: "second" }, capabilities: {} });
    let errs = 0;
    for (let i = 0; i < 25; i++) {
      const r = await rpc("tools/call", { name: "set_cell", arguments: { name: "shared", content: `from-mcp-${i}` } });
      if (r.result?.isError) errs++;
      await rpc("tools/call", { name: "set_range", arguments: { cells: { [`mcp_${i % 5}`]: String(i) } } });
    }
    p.stdin.end();
    p.kill("SIGINT");
    return errs;
  })();

  for (let r = 2; r <= 16; r++) {
    await cell(app.page, `C${r}`).click();
    await app.page.keyboard.type(`typed-${r}`);
    await app.page.keyboard.press("Enter");
  }

  await cli;
  const mcpErrors = await mcp;
  await until(async () => {
    const b = await app.call("get_board");
    return agents.every((n) => b.cells.find((c) => c.name === n)?.state === "ok");
  }, { timeout: 120000, what: "every agent cell to finish" });
  await app.page.waitForTimeout(2000);
  console.log(`busy for ${Math.round((Date.now() - t0) / 1000)} s; second MCP client errors: ${mcpErrors}`);

  const b = await app.call("get_board");
  const by = (n) => b.cells.find((c) => c.name === n);
  // `shared` must be one writer's whole value.
  const shared = by("shared").value;
  assert.match(shared, /^from-(B[2-7]|cli-\d+|mcp-\d+)$/, `a whole value: ${JSON.stringify(shared)}`);
  for (const n of agents) assert.equal(by(`own_${n}`)?.value, "done", `own_${n}`);
  for (const n of agents) assert.equal(by(n).value, `ok-${n}`, n);
  assert.equal(by("cli_done")?.value, "yes");
  for (let i = 0; i < 5; i++) assert.ok(by(`mcp_${i}`), `mcp_${i}`);
  for (let r = 2; r <= 16; r++) assert.equal(by(`C${r}`)?.value, `typed-${r}`, `C${r} kept what the person typed`);
  const drawn = await app.page.evaluate(() => [...document.querySelectorAll("#cells .cell[data-name]")].map((e) => [e.dataset.name, e.textContent]));
  let disagree = [];
  for (const c of b.cells) {
    const f = join(dir, ".recalc/cells", c.name);
    const disk = existsSync(f) ? readFileSync(f, "utf8").replace(/\n$/, "") : "";
    if (disk !== c.value) disagree.push(`${c.name}: engine ${JSON.stringify(c.value)} disk ${JSON.stringify(disk)}`);
  }
  for (const [n, text] of drawn) {
    const c = by(n);
    if (c && c.value && !text.includes(c.value.split("\n")[0].slice(0, 20))) disagree.push(`${n}: drawn ${JSON.stringify(text)} engine ${JSON.stringify(c.value)}`);
  }
  assert.deepEqual(disagree, [], "window, engine and disk agree");
  const stuck = b.cells.filter((c) => ["computing", "queued"].includes(c.state)).map((c) => c.name);
  assert.deepEqual(stuck, [], "nothing stuck");
  assert.deepEqual(app.errors, [], "no page errors");
  console.log(`ok: ${b.cells.length} cells consistent; shared = ${shared}`);
} finally {
  await app.close();
}
