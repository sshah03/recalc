// Copies the xterm.js UMD bundles + css out of node_modules into vendor/,
// which is what index.html loads. No bundler: the UMD files set window.Terminal
// and window.FitAddon, and app.ts uses them through ambient declarations.

import { mkdirSync, copyFileSync } from "node:fs";

const dst = new URL("vendor/xterm/", import.meta.url);
mkdirSync(dst, { recursive: true });

const files = [
  ["@xterm/xterm/lib/xterm.js", "xterm.js"],
  ["@xterm/xterm/css/xterm.css", "xterm.css"],
  ["@xterm/addon-fit/lib/addon-fit.js", "addon-fit.js"],
];
for (const [from, to] of files) {
  copyFileSync(new URL(`node_modules/${from}`, import.meta.url), new URL(to, dst));
}
console.log("vendored xterm into vendor/xterm/");
