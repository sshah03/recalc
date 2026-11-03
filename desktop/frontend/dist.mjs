// Copies what index.html loads into dist/, the folder the app bundles, so
// node_modules, sources and tests stay out of it. bridge-host.html is the
// test bridge's hidden page; in a release build it has nothing to call.

import { cpSync, rmSync } from "node:fs";

const dist = new URL("dist/", import.meta.url);
rmSync(dist, { recursive: true, force: true });
for (const path of ["index.html", "bridge-host.html", "styles.css", "favicon.svg", "js/", "vendor/"]) {
  cpSync(new URL(path, import.meta.url), new URL(path, dist), { recursive: true });
}
console.log("assembled dist/");
