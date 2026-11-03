// install.sh against a stand-in for GitHub's releases API: a disk image made
// from this checkout's release build, installed into a home and an
// Applications folder of the check's own. macOS. Free. Needs a release
// bundle (`cargo tauri build --bundles app` in desktop/src-tauri), then
// `node install.mjs [filter]`. Linux runs in Docker when RECALC_E2E_DOCKER=1.
import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, readlinkSync, realpathSync, symlinkSync, writeFileSync, cpSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "../..");
const script = join(root, "install.sh");
const app = join(root, "target/release/bundle/macos/recalc.app");
const results = [];
const only = process.argv[2];
async function check(name, f) {
  if (only && !name.includes(only)) return;
  try {
    await f();
    results.push([name, "ok"]);
    console.log(`ok   ${name}`);
  } catch (e) {
    results.push([name, "FAIL"]);
    console.log(`FAIL ${name}\n     ${String(e.stack ?? e).split("\n").slice(0, 6).join("\n     ")}`);
  }
}

// ---- a release, served ----
const dist = realpathSync(mkdtempSync(join(tmpdir(), "recalc-dist-")));
const arch = process.arch === "arm64" ? "aarch64" : "x64";
const dmg = join(dist, `recalc_0.1.0_${arch}.dmg`);
mkdirSync(join(dist, "stage"));
cpSync(app, join(dist, "stage/recalc.app"), { recursive: true, verbatimSymlinks: true });
execFileSync("hdiutil", ["create", "-quiet", "-volname", "recalc", "-srcfolder", join(dist, "stage"), "-format", "UDZO", "-ov", dmg]);
writeFileSync(join(dist, "recalc_0.1.0_amd64.AppImage"), "#!/bin/sh\necho \"recalc 0.1.0 (appimage) $*\"\n");
let hits = [];
const server = createServer((req, res) => {
  hits.push(req.url);
  const base = `http://${req.headers.host}`;
  const release = (tag) =>
    JSON.stringify({
      tag_name: tag,
      assets: [`recalc_0.1.0_${arch}.dmg`, "recalc_0.1.0_amd64.AppImage", "recalc_0.1.0_amd64.deb"].map((n) => ({ name: n, browser_download_url: `${base}/download/${n}` })),
    }); // on one line: the script mustn't depend on GitHub's layout
  if (req.url === "/repos/me/recalc/releases/latest") return res.end(release("v0.1.0"));
  if (req.url === "/repos/me/recalc/releases/tags/v0.1.0") return res.end(release("v0.1.0"));
  if (req.url.startsWith("/download/")) {
    const f = join(dist, req.url.slice("/download/".length));
    if (existsSync(f)) return res.end(readFileSync(f));
  }
  res.statusCode = 404;
  res.end("{}");
});
await new Promise((r) => server.listen(0, "0.0.0.0", r));
const port = server.address().port;

/** Run install.sh as someone with this home and PATH would. Not spawnSync:
 * the stand-in API is served from this process. */
function install(home, { env = {}, path } = {}) {
  const child = spawn("sh", [script], {
    env: {
      HOME: home,
      PATH: path ?? `${home}/.local/bin:/usr/bin:/bin:/usr/sbin:/sbin`,
      RECALC_API: `http://127.0.0.1:${port}`,
      RECALC_REPO: "me/recalc",
      RECALC_APPS: join(home, "Applications"),
      ...env,
    },
  });
  let out = "";
  child.stdout.on("data", (d) => (out += d));
  child.stderr.on("data", (d) => (out += d));
  return new Promise((resolve) => child.on("close", (code) => resolve({ code, out })));
}
/** A command's output, without blocking the server. */
function run(cmd, args) {
  const child = spawn(cmd, args);
  let out = "";
  child.stdout.on("data", (d) => (out += d));
  child.stderr.on("data", (d) => (out += d));
  return new Promise((resolve, reject) => child.on("close", (code) => (code === 0 ? resolve(out) : reject(new Error(`${cmd} exited ${code}:\n${out}`)))));
}
const newHome = () => realpathSync(mkdtempSync(join(tmpdir(), "recalc-home-")));

await check("installs the latest release: recalc.app in Applications, `recalc` on the PATH, runnable, not quarantined", async () => {
  const home = newHome();
  const r = await install(home);
  assert.equal(r.code, 0, r.out);
  const installed = join(home, "Applications/recalc.app");
  assert.ok(existsSync(join(installed, "Contents/MacOS/recalc")), r.out);
  const link = join(home, ".local/bin/recalc");
  assert.equal(readlinkSync(link), join(installed, "Contents/MacOS/recalc"));
  assert.match(execFileSync(link, ["--version"], { encoding: "utf8" }), /^recalc 0\.1\.0/);
  const xattrs = execFileSync("xattr", ["-r", installed], { encoding: "utf8" });
  assert.doesNotMatch(xattrs, /com\.apple\.quarantine/);
  execFileSync("codesign", ["--verify", "--deep", installed]);
  assert.match(r.out, /Installed recalc v0\.1\.0/);
  assert.doesNotMatch(r.out, /Add .* to your PATH/, "~/.local/bin was on the PATH");
  assert.ok(hits.includes("/repos/me/recalc/releases/latest"));
});

await check("installing again replaces the app; a pinned version asks for that tag", async () => {
  const home = newHome();
  assert.equal((await install(home)).code, 0);
  writeFileSync(join(home, "Applications/recalc.app/Contents/old-marker"), "x");
  hits = [];
  const r = await install(home, { env: { RECALC_VERSION: "v0.1.0" } });
  assert.equal(r.code, 0, r.out);
  assert.ok(hits.includes("/repos/me/recalc/releases/tags/v0.1.0"));
  assert.ok(!existsSync(join(home, "Applications/recalc.app/Contents/old-marker")), "a fresh copy, not a merge");
});

await check("an old `recalc` link is replaced; a real program is left alone and said so; a folder off the PATH is said so", async () => {
  const home = newHome();
  mkdirSync(join(home, ".local/bin"), { recursive: true });
  symlinkSync("/bin/echo", join(home, ".local/bin/recalc"));
  let r = await install(home);
  assert.equal(r.code, 0, r.out);
  assert.match(readlinkSync(join(home, ".local/bin/recalc")), /recalc\.app\/Contents\/MacOS\/recalc$/);

  const other = newHome();
  mkdirSync(join(other, ".local/bin"), { recursive: true });
  writeFileSync(join(other, ".local/bin/recalc"), "#!/bin/sh\necho mine\n", { mode: 0o755 });
  r = await install(other);
  assert.equal(r.code, 0, r.out);
  assert.match(r.out, /Left .*\.local\/bin\/recalc alone/);
  assert.equal(readFileSync(join(other, ".local/bin/recalc"), "utf8"), "#!/bin/sh\necho mine\n");
  assert.ok(lstatSync(join(other, ".local/bin/recalc")).isFile());

  const third = newHome();
  r = await install(third, { path: "/usr/bin:/bin:/usr/sbin:/sbin" });
  assert.equal(r.code, 0, r.out);
  assert.match(r.out, /Add .*\.local\/bin to your PATH/);
  assert.ok(existsSync(join(third, ".local/bin/recalc")));
});

await check("a missing release or a release without this Mac's file stops with a message, and installs nothing", async () => {
  const home = newHome();
  let r = await install(home, { env: { RECALC_VERSION: "v9.9.9" } });
  assert.notEqual(r.code, 0);
  assert.match(r.out, /couldn't read .*releases\/tags\/v9\.9\.9/);
  r = await install(home, { env: { RECALC_REPO: "nobody/nothing" } });
  assert.notEqual(r.code, 0);
  assert.ok(!existsSync(join(home, "Applications/recalc.app")));
});

await check("the Linux steps: the AppImage goes to ~/.local/share/recalc and `recalc` runs it (uname says Linux here)", async () => {
  const home = newHome();
  const shim = join(home, "shim");
  mkdirSync(shim);
  writeFileSync(join(shim, "uname"), '#!/bin/sh\ncase "$1" in -s) echo Linux ;; -m) echo x86_64 ;; *) echo Linux ;; esac\n', { mode: 0o755 });
  const r = await install(home, { path: `${shim}:${home}/.local/bin:/usr/bin:/bin:/usr/sbin:/sbin` });
  assert.equal(r.code, 0, r.out);
  assert.match(r.out, /Installed recalc v0\.1\.0 in .*\.local\/share\/recalc \(Linux support is experimental\)/);
  assert.equal(readlinkSync(join(home, ".local/bin/recalc")), join(home, ".local/share/recalc/recalc.AppImage"));
  assert.match(execFileSync(join(home, ".local/bin/recalc"), ["--version"], { encoding: "utf8" }), /recalc 0\.1\.0 \(appimage\) --version/);
  assert.ok(!existsSync(join(home, "Applications/recalc.app")), "no Mac app on Linux");
});

if (process.env.RECALC_E2E_DOCKER) {
  await check("Linux: the AppImage goes to ~/.local/share/recalc, `recalc` runs it", async () => {
    const out = await run("docker", ["run", "--rm", "-v", `${script}:/install.sh:ro`, "-e", `RECALC_API=http://host.docker.internal:${port}`, "-e", "RECALC_REPO=me/recalc", "debian:bookworm-slim", "sh", "-c",
      "apt-get update -qq >/dev/null && apt-get install -y -qq curl >/dev/null && export HOME=/root PATH=/root/.local/bin:$PATH && sh /install.sh && recalc --version"]);
    assert.match(out, /Installed recalc v0\.1\.0 in \/root\/\.local\/share\/recalc/);
    assert.match(out, /recalc 0\.1\.0 \(appimage\) --version/);
  });
}

server.close();
const failed = results.filter(([, r]) => r !== "ok").length;
console.log(`\n${results.length - failed} of ${results.length} passed`);
process.exit(failed ? 1 : 0);
