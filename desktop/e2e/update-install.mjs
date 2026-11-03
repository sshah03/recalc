// Restart to Update, for real: builds this checkout as a 0.1.0 app and a
// signed 0.1.1 update, serves the update from here, has the 0.1.0 app install
// it, and checks the app comes back as 0.1.1 with its board. macOS. Free, but
// two app builds (a few minutes). Needs the update key
// (~/.tauri/recalc-updater.key, or TAURI_SIGNING_PRIVATE_KEY).
// `node update-install.mjs`
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { cpSync, mkdtempSync, readFileSync, realpathSync, rmSync } from "node:fs";
import { createServer } from "node:http";
import { homedir, tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const tauri = join(here, "../src-tauri");
const bundles = join(here, "../../target/debug/bundle/macos");
const key = process.env.TAURI_SIGNING_PRIVATE_KEY ?? readFileSync(join(homedir(), ".tauri/recalc-updater.key"), "utf8");
// The real path: the updater refuses to run from behind a symlink (/var → /private/var).
const scratch = realpathSync(mkdtempSync(join(tmpdir(), "recalc-update-")));
const build = (config, env = {}) =>
  execFileSync("cargo", ["tauri", "build", "--debug", "--bundles", "app", "--config", JSON.stringify(config)], {
    cwd: tauri,
    env: { ...process.env, ...env },
    stdio: ["ignore", "ignore", "inherit"],
  });
const plistVersion = (app) => execFileSync("/usr/libexec/PlistBuddy", ["-c", "Print :CFBundleShortVersionString", join(app, "Contents/Info.plist")], { encoding: "utf8" }).trim();

console.log("building the 0.1.1 update…");
build({ version: "0.1.1", bundle: { createUpdaterArtifacts: true } }, { TAURI_SIGNING_PRIVATE_KEY: key, TAURI_SIGNING_PRIVATE_KEY_PASSWORD: "" });
cpSync(join(bundles, "recalc.app.tar.gz"), join(scratch, "recalc.app.tar.gz"));
const signature = readFileSync(join(bundles, "recalc.app.tar.gz.sig"), "utf8");
console.log("building the 0.1.0 app…");
build({ version: "0.1.0" });
const installed = join(scratch, "Applications/recalc.app");
cpSync(join(bundles, "recalc.app"), installed, { recursive: true });
assert.equal(plistVersion(installed), "0.1.0");

const server = createServer((req, res) => {
  const port = server.address().port;
  if (req.url === "/latest.json") {
    const plat = { signature, url: `http://127.0.0.1:${port}/recalc.app.tar.gz` };
    res.end(JSON.stringify({ version: "0.1.1", notes: "The test update.", pub_date: new Date().toISOString(), platforms: { "darwin-aarch64": plat, "darwin-x86_64": plat } }));
  } else if (req.url === "/recalc.app.tar.gz") {
    res.end(readFileSync(join(scratch, "recalc.app.tar.gz")));
  } else {
    res.statusCode = 404;
    res.end();
  }
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));

// lib.mjs reads RECALC_BIN when it loads: the installed 0.1.0 app.
process.env.RECALC_BIN = join(installed, "Contents/MacOS/recalc");
const { board, open, until } = await import("./lib.mjs");
const dir = board();
const app = await open(dir, { env: { RECALC_UPDATE_URL: `http://127.0.0.1:${server.address().port}/latest.json`, RECALC_UPDATE_DELAY_MS: "3600000" } });
let ok = false;
try {
  await app.call("set_cell", { name: "A1", content: "kept across the update" });
  assert.equal((await app.call("update_status")).current, "0.1.0");
  const found = await app.call("check_update");
  assert.equal(found.version, "0.1.1");
  assert.equal(found.can_install, true);
  // Installs, then restarts: this call never answers.
  void app.call("install_update").catch(() => {});
  await until(() => plistVersion(installed) === "0.1.1", { timeout: 60000, what: "the bundle replaced" });
  // The restarted app runs from the same place, with the same bridge port.
  await until(async () => {
    try {
      return (await app.call("update_status")).current === "0.1.1";
    } catch {
      return false;
    }
  }, { timeout: 30000, what: "the app back as 0.1.1" });
  const cells = (await app.call("get_board")).cells;
  assert.equal(cells.find((c) => c.name === "A1")?.value, "kept across the update");
  ok = true;
  console.log("ok   Restart to Update installed 0.1.1 over 0.1.0 and the app came back with its board");
} catch (e) {
  console.log(`FAIL ${e.stack ?? e}`);
} finally {
  await app.close();
  // The restarted process isn't the one lib.mjs started.
  try {
    execFileSync("pkill", ["-f", join(installed, "Contents/MacOS/recalc")]);
  } catch {}
  server.close();
  if (!process.env.RECALC_E2E_KEEP) rmSync(scratch, { recursive: true, force: true });
}
process.exit(ok ? 0 : 1);
