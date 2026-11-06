// A first run that says what's missing, the command line tool, and updates.
// Free: stand-in `claude` and `codex`, a home folder of the check's own, and a
// release feed served from here. `cargo build -p recalc-desktop`, then
// `node onboard.mjs [filter]`.
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, readlinkSync, realpathSync, symlinkSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { board, open, recalc, until } from "./lib.mjs";

process.setMaxListeners(50);
const require = createRequire(import.meta.url);
const here = dirname(fileURLToPath(import.meta.url));

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

const scratch = mkdtempSync(join(tmpdir(), "recalc-onboard-"));
/** A stand-in CLI that answers its status subcommand as given. */
function standIn(name, { out, code }) {
  const file = join(scratch, `${name}-${Math.random().toString(36).slice(2)}`);
  writeFileSync(file, `#!/bin/sh\nprintf '%s\\n' '${out}'\nexit ${code}\n`, { mode: 0o755 });
  return file;
}
const CLAUDE_IN = standIn("claude", { out: '{"loggedIn": true, "authMethod": "claude.ai"}', code: 0 });
const CLAUDE_OUT = standIn("claude", { out: '{"loggedIn": false, "authMethod": "none"}', code: 1 });
const CODEX_IN = standIn("codex", { out: "Logged in using ChatGPT", code: 0 });

/** A board whose agents are these programs. */
function boardWith(paths) {
  const dir = board();
  mkdirSync(join(dir, ".recalc"), { recursive: true });
  writeFileSync(join(dir, ".recalc/config.toml"), `[paths]\n${Object.entries(paths).map(([k, v]) => `${k} = "${v}"`).join("\n")}\n`);
  return dir;
}
/** A home folder of the check's own, and a login shell whose PATH is `path` (`~` is that home). */
function home(path) {
  const h = mkdtempSync(join(tmpdir(), "recalc-home-"));
  const shell = join(h, "fake-shell");
  const p = path.map((d) => d.replace(/^~/, h)).join(":");
  // Called as `$SHELL -i -l -c <script>`.
  writeFileSync(shell, `#!/bin/sh\nPATH='${p}'\nexport PATH\nshift 3\nexec /bin/sh -c "$1"\n`, { mode: 0o755 });
  return { HOME: h, SHELL: shell, dir: h };
}
const startScreen = async (app) => {
  await app.page.waitForSelector("#overlay:not([hidden]) .start", { timeout: 8000 });
};
const note = (app) => app.page.locator("#overlay .agent-note");

await check("the start screen says no agent is installed, with a way to get each", async () => {
  const app = await open(boardWith({ claude: "/nonexistent/claude", codex: "/nonexistent/codex", pi: "/nonexistent/pi" }));
  try {
    await startScreen(app);
    await until(() => note(app).isVisible(), { what: "the agent note" });
    const text = await note(app).textContent();
    assert.match(text, /Install Claude Code, Codex or pi/);
    assert.match(text, /Claude Code\s*Not installed/);
    assert.match(text, /Codex\s*Not installed/);
    assert.match(text, /pi\s*Not installed/);
    assert.deepEqual(await note(app).locator("button").allTextContents(), ["Get Claude Code", "Get Codex", "Get pi", "Check Again"]);
    assert.ok(await app.page.locator("#overlay .start-pane:not([hidden]) .agent-note + .start-cards").count(), "above the cards");
    await app.shot("onboard-no-agents");
  } finally {
    await app.close();
  }
});

await check("an agent installed but signed out is named, with how to sign in; the ready one isn't", async () => {
  const app = await open(boardWith({ claude: CLAUDE_OUT, codex: CODEX_IN, pi: "/nonexistent/pi" }));
  try {
    await startScreen(app);
    await until(() => note(app).isVisible(), { what: "the agent note" });
    const text = await note(app).textContent();
    assert.match(text, /installed but not signed in/);
    assert.match(text, /Claude Code\s*Not signed in\. Open Terminal and run claude once\./);
    assert.doesNotMatch(text, /Codex/);
  } finally {
    await app.close();
  }
});

await check("with an agent ready, the start screen says nothing about agents", async () => {
  const app = await open(boardWith({ claude: CLAUDE_IN, codex: "/nonexistent/codex", pi: "/nonexistent/pi" }));
  try {
    await startScreen(app);
    const st = await app.call("agent_status");
    assert.deepEqual(st.map((a) => [a.name, a.signed_in]), [["claude", true], ["codex", null], ["pi", null]]);
    await app.page.waitForTimeout(500);
    assert.ok(await note(app).isHidden());
  } finally {
    await app.close();
  }
});

const cliDialog = (app) => app.page.locator("#overlay .onboard");
await check("Install Command Line Tool links recalc where a terminal looks, replaces an old link only when asked, and leaves a real program alone", async () => {
  const h = home(["~/.local/bin", "/usr/bin", "/bin"]);
  const app = await open(board(), { env: { HOME: h.HOME, SHELL: h.SHELL } });
  const link = join(h.dir, ".local/bin/recalc");
  const exe = realpathSync(recalc);
  try {
    await app.page.keyboard.press("Escape");
    await app.menu("app.cli");
    await until(async () => /Puts a link/.test(await cliDialog(app).textContent()), { what: "the dialog" });
    assert.match(await cliDialog(app).textContent(), new RegExp(`at ${link.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}`));
    await app.page.locator("#overlay button", { hasText: /^Install$/ }).click();
    await until(async () => /Installed at/.test(await cliDialog(app).textContent()), { what: "installed" });
    assert.equal(realpathSync(link), exe, "the link is to this app");
    assert.equal((await app.call("cli_status")).installed, true);

    // An old link to another build: named, and replaced only on Replace.
    const { rmSync } = await import("node:fs");
    rmSync(link);
    symlinkSync("/bin/echo", link);
    await app.page.keyboard.press("Escape");
    await app.menu("app.cli");
    await until(async () => /linked to \/bin\/echo/.test(await cliDialog(app).textContent()), { what: "the old link" });
    await app.page.locator("#overlay button", { hasText: /^Cancel$/ }).click();
    assert.equal(readlinkSync(link), "/bin/echo", "Cancel leaves it");
    await app.menu("app.cli");
    await until(() => app.page.locator("#overlay button", { hasText: /^Replace$/ }).isVisible(), { what: "Replace" });
    await app.page.locator("#overlay button", { hasText: /^Replace$/ }).click();
    await until(() => realpathSync(link) === exe, { what: "the replaced link" });

    // A real program is never removed.
    rmSync(link);
    writeFileSync(link, "#!/bin/sh\necho mine\n", { mode: 0o755 });
    const refused = await app.call("install_cli", { replace: true }).then(() => "", (e) => e.message);
    assert.match(refused, /already a recalc program at .*Remove or rename it/);
    assert.ok(lstatSync(link).isFile() && readFileSync(link, "utf8").includes("mine"), "untouched");
  } finally {
    await app.close();
  }
});

// ---- updates ----

/** A signature made with someone else's key: well formed, and not ours. */
let foreign = null;
function foreignSignature(bundle) {
  if (foreign) return foreign;
  const { execFileSync } = require("node:child_process");
  const dir = mkdtempSync(join(tmpdir(), "recalc-key-"));
  const file = join(dir, "recalc.app.tar.gz");
  writeFileSync(file, bundle);
  const tauri = join(here, "../src-tauri");
  execFileSync("cargo", ["tauri", "signer", "generate", "--ci", "-w", join(dir, "k")], { cwd: tauri, stdio: "ignore" });
  execFileSync("cargo", ["tauri", "signer", "sign", "-f", join(dir, "k"), "-p", "", file], { cwd: tauri, stdio: "ignore" });
  foreign = readFileSync(`${file}.sig`, "utf8").trim();
  return foreign;
}

/** A release feed: `latest.json` says `version`, with a bundle signed by another key. */
async function feed(version) {
  const bundle = Buffer.from("not a real bundle");
  const signature = foreignSignature(bundle);
  const server = createServer((req, res) => {
    if (req.url === "/latest.json") {
      const plat = { signature, url: `http://127.0.0.1:${server.address().port}/recalc.app.tar.gz` };
      res.setHeader("content-type", "application/json");
      res.end(JSON.stringify({
        version,
        notes: "Fixes the thing.\nAnd another.",
        pub_date: "2026-10-14T12:00:00Z",
        platforms: Object.fromEntries(["darwin-aarch64", "darwin-x86_64", "linux-x86_64", "darwin-aarch64-app", "darwin-x86_64-app"].map((k) => [k, plat])),
      }));
    } else if (req.url === "/recalc.app.tar.gz") {
      res.end(bundle);
    } else {
      res.statusCode = 404;
      res.end();
    }
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  return { url: `http://127.0.0.1:${server.address().port}/latest.json`, close: () => server.close() };
}
/** The app's own version, as the update dialog states it. */
const VERSION = JSON.parse(readFileSync(join(here, "../src-tauri/tauri.conf.json"), "utf8")).version;
const updates = (app) => app.page.locator("#overlay .onboard");

await check("Check for Updates: built from source it says so; on the newest version it says so", async () => {
  const h = home(["/usr/bin", "/bin"]);
  let app = await open(board(), { env: { HOME: h.HOME }, unset: ["RECALC_UPDATE_URL"] });
  try {
    await app.page.keyboard.press("Escape");
    await app.menu("app.updates");
    await until(async () => /built from source/.test(await updates(app).textContent()), { what: "built from source" });
  } finally {
    await app.close();
  }
  const f = await feed("0.0.1");
  app = await open(board(), { env: { HOME: h.HOME, RECALC_UPDATE_URL: f.url } });
  try {
    await app.page.keyboard.press("Escape");
    await app.menu("app.updates");
    await until(async () => new RegExp(`You have the newest version, recalc ${VERSION.replaceAll(".", "\\.")}`).test(await updates(app).textContent()), { what: "up to date" });
  } finally {
    await app.close();
    f.close();
  }
});

await check("an update found by the daily check shows in the status bar; its notes are shown; an update signed with another key is refused and the app carries on", async () => {
  const h = home(["/usr/bin", "/bin"]);
  const f = await feed("9.9.9");
  const app = await open(board(), { env: { HOME: h.HOME, RECALC_UPDATE_URL: f.url, RECALC_UPDATE_DELAY_MS: "4000" } });
  try {
    await app.page.keyboard.press("Escape");
    await until(() => app.page.locator("#status-update").isVisible(), { what: "the status bar's update", timeout: 20000 });
    assert.equal(await app.page.locator("#status-update").textContent(), "Update to 9.9.9");
    await app.page.click("#status-update");
    await until(async () => new RegExp(`recalc 9\\.9\\.9 is available\\. You have ${VERSION.replaceAll(".", "\\.")}\\.`).test(await updates(app).textContent()), { what: "the dialog" });
    assert.equal(await app.page.locator("#overlay .update-notes").textContent(), "Fixes the thing.\nAnd another.");
    await app.call("set_cell", { name: "A1", content: "still here" });
    await app.page.locator("#overlay button", { hasText: /^Restart to Update$/ }).click();
    await until(async () => /didn't install/.test(await app.page.locator("#overlay .update-progress").textContent()), { what: "the refusal", timeout: 20000 });
    assert.match(await app.page.locator("#overlay .update-progress").textContent(), /signature/i);
    assert.equal((await app.call("get_board")).cells.find((c) => c.name === "A1")?.value, "still here", "the app is still running its board");
  } finally {
    await app.close();
    f.close();
  }
});

await check("turning the daily check off is remembered, and then no check runs", async () => {
  const h = home(["/usr/bin", "/bin"]);
  const f = await feed("9.9.9");
  let hits = 0;
  const counting = createServer((req, res) => {
    hits++;
    fetch(f.url).then(async (r) => res.end(await r.text()));
  });
  await new Promise((r) => counting.listen(0, "127.0.0.1", r));
  const url = `http://127.0.0.1:${counting.address().port}/latest.json`;
  let app = await open(board(), { env: { HOME: h.HOME, RECALC_UPDATE_URL: url } });
  try {
    await app.page.keyboard.press("Escape");
    await app.menu("app.updates");
    const box = app.page.locator("#overlay .onboard-check input");
    await until(() => box.isVisible(), { what: "the checkbox" });
    assert.equal(await box.isChecked(), true, "on by default");
    await box.uncheck();
    await until(() => existsSync(join(h.dir, "Library/Application Support/dev.recalc.desktop/settings.json")) || existsSync(join(h.dir, ".config/dev.recalc.desktop/settings.json")), { what: "the setting saved" });
  } finally {
    await app.close();
  }
  hits = 0;
  app = await open(board(), { env: { HOME: h.HOME, RECALC_UPDATE_URL: url, RECALC_UPDATE_DELAY_MS: "300" } });
  try {
    await app.page.waitForTimeout(2500);
    assert.equal(hits, 0, "no check with it off");
    assert.equal(await app.page.locator("#status-update").count(), 0);
  } finally {
    await app.close();
    counting.close();
    f.close();
  }
});

const failed = results.filter(([, r]) => r !== "ok").length;
console.log(`\n${results.length - failed} of ${results.length} passed`);
process.exit(failed ? 1 : 0);
