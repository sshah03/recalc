// recalc's connection for pi (https://pi.dev). recalc writes this file into a
// board's .recalc folder and starts pi with `-e <this file> --recalc-mcp
// <config>`, where the config is the board's MCP config (Claude Code's
// format). It connects pi to the board's tools and gives the model recalc's
// instructions. recalc rewrites it on every run, so changes here don't last.
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";

export default function (pi: any) {
  pi.registerFlag("recalc-mcp", {
    description: "The recalc board's MCP config (recalc passes this)",
    type: "string",
  });
  // Flags are parsed only after extensions load, and a server registered now
  // connects before the first prompt, so read the path from argv.
  const at = process.argv.indexOf("--recalc-mcp");
  const config = at >= 0 ? process.argv[at + 1] : undefined;
  if (!config) return;
  const server = JSON.parse(readFileSync(config, "utf8")).mcpServers.recalc;
  // pi starts MCP servers with a minimal environment; recalc's server needs
  // RECALC_AGENT (the cell a session works for) and the rest of recalc's.
  const env: Record<string, string> = {};
  for (const [k, v] of Object.entries(process.env)) if (v !== undefined) env[k] = v;
  // Direct: the tools are declared to the model, as Claude Code and Codex see them.
  pi.registerMcpServer("recalc", {
    ...server,
    env,
    exposure: "direct",
    description: "The recalc board in this folder: read and write its cells",
  });
  // pi lists a server's instructions only for tools it reaches through
  // codemode, so add recalc's to the system prompt.
  let instructions = "";
  try {
    instructions = readFileSync(join(dirname(config), "prompts", "instructions.md"), "utf8").trim();
  } catch {}
  if (instructions) {
    pi.on("before_agent_start", (event: any) => {
      event.systemPromptOptions.sections.recalc = instructions;
    });
  }
}
