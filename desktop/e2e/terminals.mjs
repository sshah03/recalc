// Open in Terminal through each named app: joining a live session, and
// resuming a finished run. Free (stand-in agent), but it opens real terminal
// windows, so it's run by hand: `node terminals.mjs Terminal iTerm Ghostty WezTerm kitty`.
// Quit an app first to try it from a cold start.
import { writeFileSync, mkdirSync, readFileSync, existsSync, chmodSync, mkdtempSync } from "node:fs";
import { join, dirname } from "node:path";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";
import { execFileSync } from "node:child_process";
import { board, open, until } from "./lib.mjs";
const here = dirname(fileURLToPath(import.meta.url));
const tmux = (...a) => {
  try {
    return execFileSync("tmux", ["-L", "recalc", ...a], { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] });
  } catch {
    return "";
  }
};
const sessions = () => tmux("ls", "-F", "#S").split("\n").filter(Boolean);

// The stand-in agent behind a wrapper that logs how it was started.
const scratch = mkdtempSync(join(tmpdir(), "recalc-terms-"));
const log = join(scratch, "args.log");
const claude = join(scratch, "claude");
writeFileSync(claude, `#!/bin/sh\necho "$*" >> '${log}'\nexec '${join(here, "fake-agent.py")}' "$@"\n`);
chmodSync(claude, 0o755);

let failed = false;
for (const term of process.argv.slice(2)) {
  writeFileSync(log, "");
  const dir = board();
  mkdirSync(join(dir, ".recalc"), { recursive: true });
  writeFileSync(join(dir, ".recalc/config.toml"), `[paths]\nclaude = "${claude}"\nterminal = "${term}"\n`);
  // The stand-in keeps its conversations in a Claude config folder of its own.
  const app = await open(dir, { env: { CLAUDE_CONFIG_DIR: join(scratch, "claude") } });
  const cell = async (n) => (await app.call("get_board")).cells.find((c) => c.name === n);
  const handoff = (name) => app.call("session_handoff", { name, open: true }).then(() => null, (e) => String(e).slice(0, 120));
  let live, resumed;
  try {
    // A live session: the app must become a second tmux client of it.
    await app.call("set_cell", { name: "B2", content: "@claude('fake: answer=x')" });
    const before = sessions();
    await app.call("session_open", { name: "B2", cols: 80, rows: 24 });
    const s = await until(() => sessions().find((n) => n.endsWith("-B2") && !before.includes(n)), { what: "B2's session" });
    live =
      (await handoff("B2")) ??
      ((await until(() => tmux("list-clients", "-t", s).split("\n").filter(Boolean).length >= 2, { timeout: 20000 }).catch(() => false)) ? "yes" : "no");
    await app.call("session_end", { name: "B2" });

    // A finished run: the agent must be started again with --resume <its id>.
    await app.call("set_cell", { name: "B3", content: "@claude('fake: answer=y')" });
    await app.call("trigger", { name: "B3" });
    await until(async () => (await cell("B3"))?.state === "ok", { what: "B3's run" });
    const id = (await cell("B3")).session_id;
    resumed =
      (await handoff("B3")) ??
      ((await until(() => existsSync(log) && readFileSync(log, "utf8").includes("--resume " + id), { timeout: 20000 }).catch(() => false)) ? "yes" : "no");
  } finally {
    await app.close();
  }
  if (live !== "yes" || resumed !== "yes") failed = true;
  console.log(term.padEnd(9), "joins a live session:", live, "| resumes a finished run:", resumed);
}
process.exit(failed ? 1 : 0);
