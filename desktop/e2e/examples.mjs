// Reproduces the developer examples end to end: done-means-green,
// flaky-hunt, review-lanes, release-gate and beads-board.
//
//   node examples.mjs [--free] [name ...]
//
// Paid by default (about $3.50 with Claude Code): the runs that were checked by
// hand on 2026-10-04, repeated and asserted. A fix session here is
// `claude -p` with RECALC_AGENT set to the session cell, which is what the
// Stop hook keys on; in the window it is Open Session on that cell.
// --free runs no agent: what an agent would do (a fix, an answer) is done by
// hand, so the checks, the Stop hook, the gates and the precision formulas
// are exercised for nothing.
// `cargo build -p recalc-desktop` first.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, writeFileSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { board, open, recalc, until } from "./lib.mjs";

// The fix a real session made on 2026-10-04 (tests/test_invoice.py passes).
const INVOICE_FIX = `from decimal import Decimal

from .money import fmt


def _dollars(amount):
    """Decimal('1234.5') -> '$1,234.50'; negatives read as '-$1,234.50'."""
    text = f"{Decimal(fmt(amount)):,}"
    return f"-\${text[1:]}" if text.startswith("-") else f"\${text}"


def render(customer, lines):
    """lines: [(description, amount)]. Returns the invoice as text."""
    out = [f"Invoice for {customer}"]
    total = 0
    for desc, amount in lines:
        out.append(f"  {desc}: {_dollars(amount)}")
        total += amount
    out.append(f"  Total: {_dollars(total)}")
    return "\\n".join(out)
`;

const args = process.argv.slice(2);
const free = args.includes("--free");
const only = args.filter((a) => !a.startsWith("--"));
const results = [];
let spent = 0;

async function example(name, f) {
  if (only.length && !only.includes(name)) return;
  const started = Date.now();
  try {
    await f();
    results.push([name, "ok"]);
    console.log(`ok   ${name} (${((Date.now() - started) / 1000).toFixed(0)}s)`);
  } catch (e) {
    results.push([name, "FAIL"]);
    console.log(`FAIL ${name}\n     ${String(e.stack ?? e).split("\n").slice(0, 5).join("\n     ")}`);
  }
}

/** This build's `recalc` first on PATH, as the window gives its sessions. */
const env = (extra = {}) => ({ ...process.env, PATH: `${dirname(recalc)}:${process.env.PATH}`, ...extra });
const get = (dir, name) => spawnSync(recalc, ["get", name], { cwd: dir, encoding: "utf8" }).stdout.trim();

/** A Claude Code run in the board as `cell`'s session; returns its result. */
function session(dir, cell, prompt) {
  const r = spawnSync(
    "claude",
    ["-p", prompt, "--permission-mode", "acceptEdits", "--allowedTools", "Bash(python3:*)", "Read", "Edit", "--max-turns", "25", "--output-format", "json"],
    { cwd: dir, env: env({ RECALC_AGENT: cell }), encoding: "utf8", timeout: 600_000 },
  );
  const out = JSON.parse(r.stdout || "{}");
  spent += out.total_cost_usd ?? 0;
  console.log(`     ${cell}: ${out.num_turns} turns, $${(out.total_cost_usd ?? 0).toFixed(2)}: ${String(out.result ?? r.stderr).slice(0, 120)}`);
  return out;
}

/** The Stop hook as Claude Code calls it, for --free. */
function stopHook(dir, cell, hookArgs, said) {
  const r = spawnSync(recalc, ["hook", "stop", ...hookArgs], {
    cwd: dir,
    env: env({ RECALC_AGENT: cell }),
    input: JSON.stringify({ session_id: `free-${cell}`, cwd: dir, last_assistant_message: said }),
    encoding: "utf8",
  });
  return r.stdout.trim();
}

const settle = async (app, names, timeout = 300_000) =>
  until(async () => {
    const b = await app.call("get_board");
    return names.every((n) => {
      const c = b.cells.find((x) => x.name === n);
      return c && typeof c.state === "string" && !["computing", "queued", "stale"].includes(c.state);
    });
  }, { timeout, what: names.join(" ") });

// ---- 1. done-means-green ----
await example("done-means-green", async () => {
  const dir = board("done-means-green");
  const hook = ["--task", "A", "--session", "C", "--claim", "D", "--gate", "F", "--wait", "75"];
  if (free) {
    for (let i = 1; i <= 3; i++) assert.match(stopHook(dir, "B2", hook, "done"), /"decision":"block".*STALE/, `block ${i}`);
    assert.equal(stopHook(dir, "B2", hook, "done"), "", "the fourth stop goes through");
    writeFileSync(join(dir, "tally/invoice.py"), INVOICE_FIX);
    assert.equal(stopHook(dir, "B3", hook, "Done."), "", "a passing row stops at once");
  } else {
    session(dir, "B2", "You are working in the tally repository here. Reply with only the word done. Do not change any files.");
    session(dir, "B3", `${get(dir, "instructions")} Your task: invoices should show amounts the way people read them, $1,234.50; change tally.invoice.render. Its acceptance test is tests/test_invoice.py.`);
  }
  assert.equal(get(dir, "F2"), "STALE");
  assert.match(readFileSync(join(dir, ".recalc/meta/F2.toml"), "utf8"), /kind = "flagged"/, "F2 flagged after three blocks");
  assert.equal(get(dir, "F3"), "holds");
});

// ---- 2. flaky-hunt ----
await example("flaky-hunt", async () => {
  const dir = board("flaky-hunt");
  assert.ok(Number(get(dir, "B2")) > 10, `tags flakes before: ${get(dir, "B2")}/50`);
  assert.equal(get(dir, "B5"), "0", "the innocent test never fails");
  if (free) {
    writeFileSync(join(dir, "shipit/tags.py"), 'def render(labels):\n    """The labels as one line, for the release notes."""\n    return ", ".join(sorted(set(labels)))\n');
    // No window, so the hook's command-line engine re-runs the changed check.
    assert.equal(stopHook(dir, "D2", ["--task", "A", "--gate", "G", "--wait", "75"], "fixed"), "");
  } else {
    const app = await open(dir);
    try {
      await app.call("trigger", { name: "C3" });
      await settle(app, ["C3"]);
      assert.equal((await app.call("get_board")).cells.find((c) => c.name === "C3").value, "randomness");
      spent += (await app.call("get_board")).cells.find((c) => c.name === "C3").usage?.cost_usd ?? 0;
    } finally {
      await app.close();
    }
    session(dir, "D2", `${get(dir, "instructions")} Make tests.test_tags reliable. It failed ${get(dir, "B2")} of 50 runs. When you save, the board runs it 50 times again; you are done when it fails none of them.`);
  }
  assert.equal(get(dir, "E2"), "0");
  assert.equal(get(dir, "F2"), "under 6%");
  assert.equal(get(dir, "G2"), "true");
});

// ---- 3. review-lanes ----
const TRUTH = { 2: ["issue", "ok", "ok"], 3: ["ok", "issue", "ok"], 4: ["ok", "ok", "issue"], 5: ["ok", "ok", "ok"], 6: ["issue", "ok", "ok"], 7: ["ok", "issue", "ok"] };
await example("review-lanes", async () => {
  const dir = board("review-lanes");
  const config = join(dir, ".recalc/config.toml");
  writeFileSync(config, `max_agents = 6\n${existsSync(config) ? readFileSync(config, "utf8") : ""}`);
  const app = await open(dir);
  const cells = async () => (await app.call("get_board")).cells;
  try {
    const lanes = Object.keys(TRUTH).flatMap((r) => ["G", "H", "I"].map((c) => c + r));
    if (free) {
      await app.call("set_range", { cells: Object.fromEntries(lanes.map((n) => [n, TRUTH[n.slice(1)]["GHI".indexOf(n[0])]])) }).catch(async () => {
        for (const n of lanes) await app.call("set_cell", { name: n, content: TRUTH[n.slice(1)]["GHI".indexOf(n[0])] });
      });
    } else {
      await app.call("run_selection", { names: lanes, all: true });
      await settle(app, lanes, 600_000);
    }
    const cs = await cells();
    const at = (n) => cs.find((c) => c.name === n);
    const wrong = lanes.filter((n) => at(n).value !== TRUTH[n.slice(1)]["GHI".indexOf(n[0])]);
    if (!free) spent += lanes.reduce((s, n) => s + (at(n).usage?.cost_usd ?? 0), 0);
    assert.deepEqual(wrong, [], `lane answers off the planted truth: ${wrong.map((n) => `${n}=${at(n).value}`).join(", ")}`);
    if (!free) assert.ok(lanes.every((n) => at(n).cite), "every lane answer cites its diff line");
    assert.equal(at("J5").value, "ready");
    await app.call("set_mark", { names: ["G2", "G6", "H3"], kind: "verified" });
    await app.call("set_mark", { names: ["H7"], kind: "flagged", note: "looser, but it still checks the user" });
    await until(async () => (await cells()).find((c) => c.name === "tests_precision").value === "50%", { what: "tests_precision 50%" });
    assert.equal((await cells()).find((c) => c.name === "security_precision").value, "100%");
  } finally {
    await app.close();
  }
});

// ---- 4. release-gate ----
await example("release-gate", async () => {
  const dir = board("release-gate");
  const app = await open(dir);
  const v = async (n) => (await app.call("get_board")).cells.find((c) => c.name === n);
  try {
    await app.call("trigger", { name: "tag" });
    await until(async () => /not ready: 4/.test((await v("tag")).value), { what: "the tag refusing" });
    writeFileSync(join(dir, "src/acme/__init__.py"), '__version__ = "2.4.0"\n');
    writeFileSync(join(dir, "migrations/007_users_name_index.down.sql"), "DROP INDEX users_name;\n");
    await until(async () => (await v("C4")).value === "true" && (await v("C5")).value === "true", { timeout: 30_000, what: "rows 4 and 5 green on their own" });
    if (free) {
      await app.call("set_cell", { name: "B7", content: "acme 2.4.0 adds user search for admins (#211)." });
    } else {
      await app.call("trigger", { name: "B7" });
      await settle(app, ["B7"]);
      spent += (await v("B7")).usage?.cost_usd ?? 0;
      assert.match((await v("B7")).value, /#211/, "the notes name their PRs");
    }
    await app.call("set_mark", { names: ["B6", "B7"], kind: "verified" });
    await until(async () => (await v("ready")).value === "true", { what: "ready" });
    await app.call("trigger", { name: "tag" });
    await until(async () => /tagged v2\.4\.0/.test((await v("tag")).value), { what: "the tag" });
  } finally {
    await app.close();
  }
});

// ---- 5. beads-board ----
await example("beads-board", async () => {
  const dir = board("beads-board");
  const bd = (...a) => spawnSync("python3", ["tools/bd", ...a], { cwd: dir, encoding: "utf8" }).stdout;
  assert.equal(get(dir, "I2"), "closed but red", "beads says closed; its test fails");
  assert.equal(get(dir, "H3"), "waiting", "ready by checks, not by closes");
  assert.equal(get(dir, "bd_ready"), "tl-7f3.2 tl-7f3.4", "beads offers the bead after the false close");
  if (free) {
    const money = join(dir, "tally/money.py");
    writeFileSync(money, readFileSync(money, "utf8").replace("Decimal(text.strip())", 'Decimal(text.strip().replace(",", ""))'));
    writeFileSync(join(dir, "tally/invoice.py"), INVOICE_FIX);
    // No window: the hook's command-line engine re-runs the changed checks.
    for (const cell of ["J2", "J3"]) assert.equal(stopHook(dir, cell, ["--gate", "G", "--pass", "holds", "--wait", "75"], "done"), "");
  } else {
    for (const row of [2, 3]) {
      session(dir, `J${row}`, `${get(dir, "instructions")} Your bead: ${get(dir, `A${row}`)}, "${get(dir, `B${row}`)}". It is done when ${get(dir, `D${row}`)} passes: python3 -m unittest ${get(dir, `D${row}`)}.`);
    }
  }
  assert.equal(get(dir, "G2"), "holds");
  assert.equal(get(dir, "G3"), "holds");
  assert.equal(get(dir, "H4"), "ready", "export is ready once invoice holds");
  assert.match(bd("show", "tl-7f3.2", "--json"), /"status": "open"/, "the session left closing to a person");
  const app = await open(dir);
  const v = async (n) => (await app.call("get_board")).cells.find((c) => c.name === n);
  try {
    await app.call("trigger", { name: "L3" });
    await until(async () => /not closed/.test((await v("L3")).value), { what: "the close refusing before a sign-off" });
    await app.call("set_mark", { names: ["G3"], kind: "verified" });
    await until(async () => (await v("K3")).value === "true", { what: "ok to close" });
    await app.call("trigger", { name: "L3" });
    await until(async () => (await v("C3")).value === "closed", { timeout: 20_000, what: "beads saying closed" });
  } finally {
    await app.close();
  }
  assert.match(bd("show", "tl-7f3.2", "--json"), /"close_reason": "test holds; verified in recalc"/);
});

console.log(`${results.filter((r) => r[1] === "ok").length} of ${results.length} passed${free ? "" : `, $${spent.toFixed(2)} spent`}`);
process.exit(results.some((r) => r[1] === "FAIL") ? 1 : 0);
