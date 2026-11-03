# recalc desktop: development notes

The app is a Tauri window over the `recalc` core crate. The Rust side holds
one `Engine` in state, pumps filesystem changes and finished runs every
50 ms, and pushes the cells that changed to the webview; every click in the
webview is an engine call. No business logic lives in the frontend.

## Requirements

- Rust (stable) and the platform webview (WKWebView on macOS; WebKitGTK on
  Linux: `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev
  librsvg2-dev patchelf`).
- Node ≥ 20, only to compile the TypeScript UI and vendor xterm.js. No
  bundler, no framework.

## Run it

```
cd desktop/frontend && npm install && npm run build   # tsc → js/, vendors xterm
cd ../.. && cargo run -p recalc-desktop                # opens the board in the cwd
```

`npm run build` also copies the page into `frontend/dist/`, which the app
embeds when it compiles, so rebuild the app after a frontend change
(`npm run watch` only recompiles TypeScript). To watch the board fill in, drive
it from another shell (`target/debug/recalc set B2 7`, `recalc open
examples/review.recalctemplate`) or point an agent's MCP config at `recalc mcp` in
the same directory.

Live agent sessions and the driver run inside tmux when it is on PATH
(`brew install tmux`), on a server of their own: `tmux -L recalc ls` lists
them, `recalc attach <cell>` joins one. Without tmux they are plain PTYs
that end with the window.

## Checks

```
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm --prefix desktop/frontend test        # tsc, then the jsdom smoke test
```

The smoke test loads the compiled modules under jsdom with a stubbed Tauri
bridge and drives the real UI: selection, the in-place editor, autocomplete,
menu events, clipboard, deltas. It can't see layout; when a change touches
CSS or positioning, open the app and look in both appearances, since the
stylesheet carries a light and a dark token set and the terminal follows.

## End to end

`desktop/e2e` drives the real app through its UI. macOS's webview has no
automation driver, so a debug build run with `RECALC_BRIDGE=<port>` hides
its window and serves the frontend on `http://127.0.0.1:<port>`, with every
call run through the app's own IPC and every event forwarded
(`src-tauri/src/bridge.rs`; not compiled into a release). Playwright's
WebKit loads it and clicks, types, and reads the board.

```
cargo build -p recalc-desktop            # the debug app, with the bridge
cd desktop/e2e && npm ci && npx playwright install webkit
npm test                                 # the checks: no paid agents; CI runs these
npm run agents                           # a legal-demo row and a permission question, with real Claude (~$1)
npm run driver                           # the driver building a grid, with real Claude (~$0.50)
npm run examples:free                    # the developer examples, agents' steps done by hand
npm run examples                         # the same with real Claude Code (~$3.50)
```

Screenshots land in `desktop/e2e/shots/`. A native menu item is sent as
`POST /menu {id}` (a browser has no native menu), and `POST /pick {path}`
answers the next file dialog.

## Before you call it done

The hit-every-surface checklist is in [../CONTRIBUTING.md](../CONTRIBUTING.md):
menu, palette, toolbar, context menu, inspector, shortcut; MCP tool, CLI,
file contract; both agents; `types.ts`; templates; the way back out of every
state; the docs that just went stale.

## Package

```
cargo install tauri-cli --version '^2'   # once
cd desktop/src-tauri && cargo tauri build  # .app/.dmg on macOS, AppImage/.deb on Linux
```

`cargo tauri build` runs the frontend build first (`beforeBuildCommand` in
`tauri.conf.json`). The icon is drawn in `src-tauri/icons/icon.svg`; the PNGs and `icon.icns` are made from it.

## Layout

```
desktop/
  frontend/
    index.html  styles.css
    src/
      app.ts          wiring: snapshots in, renders out, keys/clipboard/menu → engine calls
      state.ts        the board store (cells map, selection, change signal)
      grid.ts         the virtualized sheet: frozen panes, row filter, find highlights, hover card, fill handle
      editor.ts       in-place editor mirrored by the formula bar; ref picking; F4 anchors
      inspector.ts    the selected cell or block: source, inputs, output, marks, session
      actions.ts      every action once — native menu ids, palette, toolbar, context menu
      chrome.ts       toolbar, status, overlays (palette, gallery, prompt), context menu
      side.ts         named cells; activity/decisions tray
      icons.ts        the toolbar's stroke icons, inline SVG
      autocomplete.ts help.ts spark.ts terminal.ts a1.ts tauri.ts types.ts
    test/smoke.mjs
  src-tauri/
    src/main.rs       entry: subcommand → CLI, none → window
    src/board.rs      the board behind the window: state, deltas, pump loop
    src/commands/     the IPC commands (sessions and boards in their own files)
    src/menu.rs       the native menu bar (ids shared with actions.ts; a test ties them)
    src/pty.rs        terminal cells and live agent sessions
    tauri.conf.json  capabilities/  icons/
```
