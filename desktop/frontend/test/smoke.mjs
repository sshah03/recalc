// Runs the compiled UI under jsdom with a stubbed Tauri bridge: checks rendering and that
// clicks, keys, menu and clipboard events become the right commands. Catches runtime
// breakage tsc can't (missing elements, bad event wiring, deltas that don't repaint).
//
//   npm run build && node test/smoke.mjs

import { readFileSync } from "node:fs";
import { JSDOM } from "jsdom";
import assert from "node:assert";

const html = readFileSync(new URL("../index.html", import.meta.url), "utf8");
const dom = new JSDOM(html, { runScripts: "outside-only", pretendToBeVisual: true });
const { window } = dom;
window.requestAnimationFrame = (fn) => setTimeout(fn, 0);

const calls = [];
const listeners = {};
const snapshot = {
  dir: "/repo",
  full: true,
  removed: [],
  activity: [{ at: Date.now(), by: "agent:a1", text: "agent:a1 set failed" }, { at: Date.now(), text: "engine ran failed -> ok" }],
  agents: { running: 1, queued: 2, max: 4 },
  cells: [
    mkCell({ name: "A1", value: "Test Suite", kind: "literal" }),
    mkCell({ name: "B2", value: "41", kind: "literal" }),
    mkCell({ name: "B3", value: "= {B2} * 2", kind: "expr", source: "= {B2} * 2", deps: ["B2"], value: "82" }),
    mkCell({ name: "A2", value: "contracts/acme.md", kind: "literal" }),
    mkCell({ name: "C2", value: "yes", kind: "agent_call", source: "@claude('{C$1} {$A2}')", deps: ["C1", "A2"],
      mark: { kind: "verified", by: "human", at: 1, note: "checked" }, provenance: { by: "agent:a1", at: 0 },
      cite: { file: "contracts/acme.md", quote: "may not assign this Agreement", where: "§12", by: "agent:a1", at: 1 },
      answer_type: { kind: "yesno", strict: true },
      usage: { input_tokens: 1000, output_tokens: 48, cost_usd: 0.02, turns: 1 },
      pending_suggestions: [{ value: "no", from: "agent:a2", at: 2 }] }),
    mkCell({ name: "D4", kind: "expr", source: "= 1 / {zero}", deps: ["zero"], value: "", state: { error: "division by zero" } }),
    mkCell({ name: "B4", kind: "agent_call", source: "@claude('{B$1} {$A4}')", value: "Delaware", provenance: { by: "agent:a1", at: 0 } }),
    mkCell({ name: "E2", kind: "agent_call", source: "@claude('{E$1} {$A2}')", value: "12 months", provenance: { by: "agent:a1", at: 0 }, session_id: "5f1e2d3c-0000-4000-8000-000000000001" }),
    mkCell({ name: "C3", value: "", kind: "agent_call", source: "@claude('{C$1} {$A3}')", deps: ["C1", "A3"], state: "queued" }),
    mkCell({ name: "E3", value: "", kind: "agent_call", source: "@claude('{E$1} {$A3}')", state: "stale" }),
    mkCell({ name: "failed", value: "2", kind: "command", source: "$ echo 2", series: true, spark: [3, 2, 2] }),
    mkCell({ name: "busy", value: "", kind: "agent_call", source: "@claude('slow')", state: "computing" }),
    mkCell({ name: "status", value: "false", kind: "expr", source: "= {failed} == 0", deps: ["failed"] }),
    mkCell({
      name: "review",
      value: "",
      kind: "agent_call",
      source: "@claude('summarize {failed}')",
      deps: ["failed"],
      state: "stale",
      session: "waiting?",
      stale_inputs: [{ name: "failed", was: "3", now: "2" }],
    }),
    mkCell({
      name: "deploy",
      value: "",
      kind: "literal",
      state: "awaiting",
      decision: {
        question: "deploy to prod?",
        options: [{ label: "yes", value: "yes" }, { label: "no", value: "no", preview: "- keep\n+ ship" }],
        allow_freetext: true,
        recommended: 1,
        asker: "agent:a1",
        posed_at: 1,
      },
    }),
  ],
};

function mkCell(o) {
  return {
    name: o.name, kind: o.kind ?? "literal", source: o.source ?? null, value: o.value ?? "",
    deps: o.deps ?? [], state: o.state ?? "ok", exit_code: null, stderr: "", duration_ms: null,
    confirmed: false, series: o.series ?? false, def_by: o.source ? (o.provenance?.by ?? "human") : null,
    decision: o.decision ?? null, pending_suggestions: o.pending_suggestions ?? [], mark: o.mark ?? null, format: o.format ?? null, cite: o.cite ?? null,
    answer_type: o.answer_type ?? null, usage: o.usage ?? null, session_id: o.session_id ?? null,
    provenance: o.provenance ?? { by: "human", at: 0 }, spark: o.spark, session: o.session,
    stale_inputs: o.stale_inputs, activity: o.activity,
  };
}

window.__TAURI__ = {
  core: {
    invoke(cmd, args) {
      calls.push({ cmd, args });
      if (globalThis.failCmd === cmd) {
        globalThis.failCmd = null;
        return Promise.reject("the agent's terminal is gone");
      }
      if (cmd === "get_board") return Promise.resolve(snapshot);
      if (cmd === "set_cell") return Promise.resolve(globalThis.setCellReply?.(args) ?? null);
      if (cmd === "needs_confirm") return Promise.resolve(false);
      if (cmd === "console") return Promise.resolve({ command: "echo 2", cwd: "/repo", watch: null, every: null });
      if (cmd === "recent_boards") return Promise.resolve(["/tmp/other-board"]);
      if (cmd === "run_selection") return Promise.resolve(globalThis.runSelectionReply?.(args) ?? { ran: ["C3"], need_confirm: [] });
      if (cmd === "delete_cells") return Promise.resolve([]);
      if (cmd === "new_board") {
        // The new board arrives as the real one does: an empty snapshot, then the reply.
        const dir = globalThis.newBoardDir ?? null;
        if (dir) listeners.board({ payload: { ...snapshot, dir, cells: [] } });
        return Promise.resolve(dir);
      }
      if (cmd === "demos")
        return Promise.resolve([{ name: "legal", blurb: "contract review" }, { name: "tax", blurb: "intake" }, { name: "bench", blurb: "benchmarks" }]);
      if (cmd === "get_layout") return Promise.resolve(JSON.stringify({ widths: { 2: 200 }, wrap: false, frozenRows: 0, frozenCols: 0, gridFrozen: true }));
      if (cmd === "stop") return Promise.resolve(true);
      if (cmd === "stop_all") return Promise.resolve(2);
      if (cmd === "launch_notices") return Promise.resolve([]);
      if (cmd === "connect_command") return Promise.resolve("claude mcp add recalc -- '/Applications/recalc.app/Contents/MacOS/recalc' mcp");
      if (cmd === "agent_paths")
        return Promise.resolve([
          { name: "claude", configured: null, found: "/Users/me/.local/bin/claude" },
          { name: "codex", configured: "/opt/codex", found: null },
        ]);
      if (cmd === "undo" || cmd === "redo") return Promise.resolve("B2");
      if (cmd === "driver_status") return Promise.resolve({ status: "none", agent: globalThis.driverAgent ?? "claude", choices: ["claude", "codex", "mine"], tmux: true });
      if (cmd === "session_handoff") return Promise.resolve("tmux -L recalc attach-session -t '=recalc-abc123-E2'");
      if (cmd === "read_source")
        return Promise.resolve({ file: args.file, kind: "text", text: "12. Assignment.\nNeither party  may not\nassign this agreement without consent.\n\n13. Notices." });
      if (cmd === "column_library")
        return Promise.resolve([{ name: "Governing law", question: "Which law governs?", kind: "text", builtin: true }]);
      if (cmd === "add_question_column") return Promise.resolve({ header: "D1", answers: ["D2", "D3"] });
      if (cmd === "model_aliases") return Promise.resolve(["fast"]);
      if (cmd === "templates")
        return Promise.resolve([
          { name: "welcome", note: "the welcome tour" },
          { name: "review", note: "documents down A, questions across" },
        ]);
      return Promise.resolve(null);
    },
  },
  event: {
    listen(ev, cb) {
      listeners[ev] = cb;
      return Promise.resolve(() => {});
    },
  },
};

// Provide the globals the ES modules read; then import the app as the page does.
globalThis.window = window;
globalThis.document = window.document;
globalThis.requestAnimationFrame = window.requestAnimationFrame;
globalThis.getComputedStyle = window.getComputedStyle.bind(window);
globalThis.MutationObserver = window.MutationObserver;
await import("../js/app.js");
const side = await import("../js/side.js");
const { board } = await import("../js/state.js");
{
  // A check named for a problem is good when false.
  const { valueClass } = await import("../js/grid.js");
  const c = (name, value) => ({ name, value, state: "ok", kind: "expr", deps: [] });
  assert.equal(valueClass(c("verified", "true")), "v-true");
  assert.equal(valueClass(c("verified", "false")), "v-false");
  assert.equal(valueClass(c("regressed", "false")), "v-true", "regressed = false reads as good");
  assert.equal(valueClass(c("contradicted", "true")), "v-false");
}

const doc = window.document;
const $ = (id) => doc.getElementById(id);
await tick();

// ---- render ----
assert.ok(doc.querySelectorAll("#cells .cell").length > 100, "grid should render a window of cells");
assert.strictEqual(doc.querySelector("#cells .cell[data-name='A1'] .val").textContent, "Test Suite");
assert.strictEqual(doc.querySelector("#cells .cell[data-name='B3'] .val").textContent, "82");
assert.ok(doc.querySelector("#cells .cell[data-name='C2']").classList.contains("mark-verified"), "verified mark on the face");
assert.strictEqual(doc.querySelector("#cells .cell[data-name='C3'] .val").textContent, "queued", "queued state shows");
assert.ok(doc.querySelector("#col-hdr .hdr.col[data-col='2']").textContent === "B", "column headers");
assert.ok(doc.querySelector("#row-hdr .hdr.row[data-row='3']"), "row headers");
assert.match($("named").textContent, /failed/);
assert.match($("named").textContent, /waiting\?/, "session badge in the named panel");
assert.match($("named").textContent, /inputs changed/, "stale-inputs badge");
assert.match($("status-agents").textContent, /1 of 4 agents running, 2 queued/);
// A usage-limit hold is labelled, not shown as a stuck queue.
listeners.board({
  payload: { dir: "/repo", full: false, removed: [], activity: [], cells: [],
    agents: { running: 0, queued: 3, max: 4, paused_until: Date.now() + 600_000 } },
});
await tick();
assert.match($("status-agents").textContent, /Agents held until .*: usage limit, 3 queued/);
listeners.board({
  payload: { dir: "/repo", full: false, removed: [], activity: [], cells: [], agents: { running: 1, queued: 2, max: 4 } },
});
await tick();
assert.strictEqual($("board-name").textContent, "repo", "board name in the title bar");
assert.match($("status-awaiting").textContent, /5 need you/, "a question, a suggestion, old inputs, an error, and a session at its prompt need you");
assert.strictEqual(doc.querySelector("#cells .cell[data-name='D4'] .val").textContent, "error: division by zero", "an empty errored cell shows its message");
assert.match(doc.querySelector("#cells .cell[data-name='C3'] .elapsed").textContent, /^\d+s$/, "a queued cell shows how long it has waited");
assert.match($("status-spend").textContent, /\$0\.02 at API prices · 1\.0k tokens/, "the spend meter is always on");
assert.ok(doc.querySelectorAll("#toolbar button").length >= 6, "toolbar buttons");
assert.ok(doc.querySelectorAll("#toolbar button svg.icon").length >= 7, "toolbar buttons carry icons");
assert.ok(calls.some((c) => c.cmd === "menu_enabled"), "native menu enabled state is pushed");

// A never-run agent is marked unlike a blank cell; its value stays empty.
assert.ok(doc.querySelector("#cells .cell[data-name='E3']").classList.contains("unrun"), "a never-run agent cell wears the placeholder");
assert.strictEqual(doc.querySelector("#cells .cell[data-name='E3'] .val").textContent, "", "the placeholder is style, not value");
assert.ok(!doc.querySelector("#cells .cell[data-name='C3']").classList.contains("unrun"), "a queued agent is not 'not run'");
assert.ok(!doc.querySelector("#cells .cell[data-name='E5']").classList.contains("unrun"), "a blank cell has no placeholder");
// Nothing selected: the inspector counts the agents still to run and offers the block.
assert.strictEqual($("inspector-title").textContent, "Inspector");
assert.match($("inspector-body").textContent, /1 agent cell hasn't run yet/, "the empty inspector counts never-run agents");
assert.match($("inspector-body").textContent, /Run selection \(⇧⌘↩\)/, "and says how to run them");
// Screen readers: a grid of cells with headers, tabs that say which is showing.
assert.strictEqual($("grid-wrap").getAttribute("role"), "grid");
assert.strictEqual(doc.querySelector("#cells .cell[data-name='B2']").getAttribute("role"), "gridcell");
assert.strictEqual(doc.querySelector("#col-hdr .hdr.col[data-col='2']").getAttribute("role"), "columnheader");
assert.strictEqual(doc.querySelector("#row-hdr .hdr.row[data-row='3']").getAttribute("role"), "rowheader");
assert.strictEqual(doc.querySelector("#tray-wrap .tabs").getAttribute("role"), "tablist");
assert.strictEqual($("tab-driver").getAttribute("aria-selected"), "true", "the showing tab is selected");
assert.strictEqual($("tab-activity").getAttribute("aria-selected"), "false");
assert.match($("tab-decisions").getAttribute("aria-label"), /Needs you, 5 waiting/, "the Needs you tab says its count");
{
  // A live session at its prompt is its own row, with ways to look at it.
  const tray = $("tray").textContent;
  const waitingRow = doc.querySelector("#tray .need-waiting");
  if (waitingRow) {
    assert.match(waitingRow.textContent, /at the prompt/);
    assert.match(waitingRow.textContent, /Show/);
    assert.match(waitingRow.textContent, /End session/);
  } else {
    assert.ok(!/at the prompt/.test(tray), "the tray shows the waiting row once it's open");
  }
}
// A column of document paths fits them; others keep the default.
assert.ok(parseInt(doc.querySelector("#col-hdr .hdr.col[data-col='1']").style.width) > 140, "the document column fits its paths");
// The empty inspector's button selects the block of agents still to run.
[...$("inspector-body").querySelectorAll("button")].find((b) => b.textContent === "Select E3").dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
await tick();
assert.strictEqual($("namebox").textContent, "E3", "Select puts the never-run agents in the selection");

// The saved layout is applied: column B is 200px wide.
assert.strictEqual(doc.querySelector("#col-hdr .hdr.col[data-col='2']").style.width, "200px", "layout widths not loaded");
// A busy board shows Stop all in the status bar; an idle one doesn't.
assert.ok(!$("status-stop").hidden, "Stop all should show while something runs");

// ---- select, edit in place, commit ----
press("#cells .cell[data-name='B2']");
await tick();
assert.strictEqual(doc.querySelector("#cells .cell[data-name='B2']").getAttribute("aria-selected"), "true", "the selected cell says so");
assert.strictEqual(doc.querySelector("#cells .cell[data-name='B3']").getAttribute("aria-selected"), "false");
assert.strictEqual($("namebox").textContent, "B2");
assert.strictEqual($("formula").value, "41");
assert.ok(doc.querySelector("#cells .cell[data-name='B3']").classList.contains("dependent"), "dependents highlighted");
assert.strictEqual($("inspector-title").textContent, "B2 · Value", "a plain title: name and kind, nothing when nothing is wrong");
// Typing starts an in-place edit seeded with the key.
key($("grid-wrap"), "7");
await tick();
assert.ok(editorOpen(), "cell editor appears");
assert.strictEqual($("cell-editor").value, "7");
assert.strictEqual($("formula").value, "7", "formula bar mirrors the editor");
$("cell-editor").value = "77";
$("cell-editor").dispatchEvent(new window.Event("input", { bubbles: true }));
assert.strictEqual($("formula").value, "77");
key($("cell-editor"), "Enter");
await tick();
const set = calls.find((c) => c.cmd === "set_cell");
assert.deepStrictEqual(set.args, { name: "B2", content: "77" }, "commit wiring wrong");
assert.strictEqual($("namebox").textContent, "B3", "Enter moves down");
assert.ok(!editorOpen(), "editor closes on commit");

// A plain value over a computed cell opens the decision, not a set.
key($("grid-wrap"), "Enter");
await tick();
$("cell-editor").value = "100";
key($("cell-editor"), "Enter");
await tick();
const edited = calls.find((c) => c.cmd === "edit_computed");
assert.deepStrictEqual(edited.args, { name: "B3", typed: "100" }, "computed-cell edit wiring wrong");
// Escape cancels.
press("#cells .cell[data-name='B2']");
key($("grid-wrap"), "x");
key($("cell-editor"), "Escape");
await tick();
assert.ok(!editorOpen());
assert.strictEqual($("formula").value, "41", "cancel restores the content");

// Dependency highlight on a formula cell; deps listed in the inspector.
press("#cells .cell[data-name='B3']");
await tick();
assert.ok(doc.querySelector("#cells .cell[data-name='B2']").classList.contains("dep"), "precedent highlighted");
assert.match($("inspector-body").textContent, /B2 = 41/, "inspector lists inputs");

// ---- autocomplete in the editor: functions, refs (with $), agents ----
press("#cells .cell[data-name='B2']");
key($("grid-wrap"), "=");
await tick();
const ed = $("cell-editor");
ed.value = "=su";
ed.setSelectionRange(3, 3);
ed.dispatchEvent(new window.Event("input", { bubbles: true }));
const acRows = [...doc.querySelectorAll("#autocomplete .ac-row .ac-name")].map((n) => n.textContent);
assert.deepStrictEqual(acRows, ["sum(range or value, ...)", "sumif(range, criterion[, sum_range])"], "autocomplete wrong");
doc.querySelector("#autocomplete .ac-row").dispatchEvent(new window.MouseEvent("mousedown", { bubbles: true }));
assert.strictEqual(ed.value, "=sum(", "autocomplete insert wrong");
ed.value = "=sum({$fa";
ed.setSelectionRange(9, 9);
ed.dispatchEvent(new window.Event("input", { bubbles: true }));
assert.match($("autocomplete").textContent, /\{failed\}/, "ref autocomplete must see through $");
// F4 cycles anchors on the ref under the caret.
ed.value = "= {B2} + 1";
ed.setSelectionRange(5, 5);
key(ed, "F4");
assert.strictEqual(ed.value, "= {$B$2} + 1", "F4 should anchor both axes");
key(ed, "F4");
assert.strictEqual(ed.value, "= {B$2} + 1");
ed.value = "@cl";
ed.setSelectionRange(3, 3);
ed.dispatchEvent(new window.Event("input", { bubbles: true }));
assert.match($("autocomplete").textContent, /@claude\('prompt'/, "agent autocomplete missing");
key(ed, "Enter");
assert.strictEqual(ed.value, "@claude('", "agent autocomplete insert wrong");
assert.ok($("autocomplete").hidden, "an accepted completion closes the dropdown");
key(ed, "Escape");
await tick();
assert.ok(!editorOpen(), "Escape leaves the editor");

// ---- typing through an input method (Japanese, Chinese, Korean, accents) ----
// Between edits the input waits, focused and unseen, on the selected cell, so a
// composition starts in it rather than the first key landing as a Latin letter.
press("#cells .cell[data-name='D9']");
await tick();
assert.strictEqual(doc.activeElement, ed, "the grid's focus waits in the cell input");
assert.ok(!editorOpen(), "unseen until typing starts");
const imeKey = (k, opts = {}) => {
  const ev = new window.KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true, ...opts });
  Object.defineProperty(ev, "keyCode", { value: 229 });
  ed.dispatchEvent(ev);
  return ev;
};
assert.ok(!imeKey("n").defaultPrevented, "a composing key is left to the input method");
assert.ok(!editorOpen(), "and doesn't seed the cell with a Latin letter");
ed.dispatchEvent(new window.CompositionEvent("compositionstart", { bubbles: true, data: "" }));
ed.value = "に";
ed.dispatchEvent(new window.Event("input", { bubbles: true }));
assert.ok(editorOpen(), "a composition starts an edit of the selected cell");
assert.strictEqual($("formula").value, "に", "with the composed text, not the key");
// Enter that accepts a conversion: WebKit ends the composition first, so the keydown
// carries only key code 229. The cell stays open; the next Enter commits.
ed.value = "日本";
ed.dispatchEvent(new window.CompositionEvent("compositionend", { bubbles: true, data: "日本" }));
imeKey("Enter");
assert.ok(editorOpen(), "Enter accepting a conversion doesn't commit the cell");
ed.dispatchEvent(new window.KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true, isComposing: true }));
assert.ok(editorOpen(), "nor does an Enter during composition");
key(ed, "Enter");
await tick();
assert.ok(!editorOpen(), "a plain Enter commits");
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "set_cell")?.args, { name: "D9", content: "日本" }, "the composed text is the cell's");
assert.strictEqual($("namebox").textContent, "D10", "and moves down");
assert.strictEqual(doc.activeElement, ed, "waiting again, on the next cell");

// ---- line breaks inside a cell ----
// A multi-line value edits with its line breaks (an input would drop them, so an
// unchanged Enter-Enter rewrote the cell flattened); ⌥Enter adds one.
listeners.board({
  payload: { dir: "/repo", full: false, removed: [], activity: [], agents: { running: 1, queued: 2, max: 4 },
    cells: [mkCell({ name: "D9", value: "line one\nline two", kind: "literal" })] },
});
await tick();
press("#cells .cell[data-name='D9']");
await tick();
const setsBefore = calls.filter((c) => c.cmd === "set_cell").length;
key($("grid-wrap"), "Enter");
await tick();
assert.ok(editorOpen(), "Enter edits");
assert.strictEqual(ed.value, "line one\nline two", "the editor keeps the value's line break");
assert.strictEqual($("formula").value, "line one\nline two", "and so does the formula bar");
key(ed, "Enter");
await tick();
assert.strictEqual(calls.filter((c) => c.cmd === "set_cell").length, setsBefore, "an unchanged multi-line value isn't rewritten");
press("#cells .cell[data-name='D9']");
await tick();
key($("grid-wrap"), "Enter");
await tick();
ed.setSelectionRange(4, 4);
key(ed, "Enter", { altKey: true });
assert.ok(editorOpen(), "⌥Enter stays in the cell");
assert.strictEqual(ed.value, "line\n one\nline two", "⌥Enter breaks the line at the caret");
key(ed, "Escape");
await tick();
listeners.board({ payload: { dir: "/repo", full: false, removed: ["D9"], activity: [], cells: [], agents: { running: 1, queued: 2, max: 4 } } });
await tick();

// ---- typed cells: a list of values, Yes or No, a date ----
const typedCell = (answer_type, value = "") =>
  listeners.board({ payload: { dir: "/repo", full: false, removed: [], activity: [], agents: { running: 1, queued: 2, max: 4 },
    cells: [mkCell({ name: "D9", value, kind: "literal", answer_type })] } });
const typedRows = () => [...$("autocomplete").querySelectorAll(".ac-row")].map((r) => (r.classList.contains("active") ? "*" : "") + r.querySelector(".ac-name").textContent);
typedCell({ kind: "choice", options: ["Approve", "Reject", "Needs info"], strict: true });
await tick();
press("#cells .cell[data-name='D9']");
await tick();
assert.ok(doc.querySelector("#cells .cell[data-name='D9'] .pick-btn"), "the selected choice cell shows ▾");
press("#cells .cell[data-name='D9'] .pick-btn");
await tick();
assert.ok(editorOpen(), "▾ opens the cell for editing");
assert.deepStrictEqual(typedRows(), ["Approve", "Reject", "Needs info"], "with its options, none picked yet");
key(ed, "Escape");
key(ed, "Enter");
await tick();
assert.ok(!editorOpen(), "Enter with nothing picked keeps the empty cell");
assert.ok(!calls.some((c) => c.cmd === "set_cell" && c.args.name === "D9" && c.args.content === "Approve"), "and picks nothing for you");
press("#cells .cell[data-name='D9']");
await tick();
key($("grid-wrap"), "r");
await tick();
assert.deepStrictEqual(typedRows(), ["*Reject"], "typing narrows the list and picks the match");
key(ed, "Enter");
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "set_cell")?.args, { name: "D9", content: "Reject" }, "Enter enters the pick");
assert.strictEqual(nameNowTyped(), "D10", "and moves down");
// Its own value lists every option, itself highlighted.
typedCell({ kind: "choice", options: ["Approve", "Reject", "Needs info"], strict: true }, "Reject");
await tick();
press("#cells .cell[data-name='D9']");
await tick();
key($("grid-wrap"), "Enter");
await tick();
assert.deepStrictEqual(typedRows(), ["Approve", "*Reject", "Needs info"]);
key(ed, "ArrowDown");
const pickRows = [...$("autocomplete").querySelectorAll(".ac-row")];
pickRows[2].dispatchEvent(new window.MouseEvent("mousedown", { bubbles: true, cancelable: true }));
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "set_cell")?.args, { name: "D9", content: "Needs info" }, "a click on a value enters it");
assert.ok(!editorOpen());
// Yes or No.
typedCell({ kind: "yesno", strict: true });
await tick();
press("#cells .cell[data-name='D9']");
await tick();
key($("grid-wrap"), "n");
await tick();
assert.deepStrictEqual(typedRows(), ["*No"]);
key(ed, "Tab");
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "set_cell")?.args, { name: "D9", content: "No" }, "Tab enters it too");
assert.strictEqual(nameNowTyped(), "E9", "and moves right");
// A date: typed in words, entered as YYYY-MM-DD; the calendar is offered.
typedCell({ kind: "date", strict: true });
await tick();
press("#cells .cell[data-name='D9']");
await tick();
for (const ch of "oct 3 2026") key(ch === " " ? ed : $("grid-wrap"), ch);
await tick();
assert.deepStrictEqual(typedRows(), ["*2026-10-03", "Choose from a calendar…"], "the date you typed, as the cell takes it");
key(ed, "Enter");
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "set_cell")?.args, { name: "D9", content: "2026-10-03" });
// A value that doesn't fit is kept, and the inspector says what the cell takes.
globalThis.setCellReply = (a) => (a.content === "soon" ? "D9 takes a date as YYYY-MM-DD; kept as you typed it" : null);
press("#cells .cell[data-name='D9']");
await tick();
for (const ch of "soon") key($("grid-wrap"), ch);
await tick();
assert.deepStrictEqual(typedRows(), ["Choose from a calendar…"], "nothing to pick: Enter keeps it");
key(ed, "Enter");
await tick();
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "set_cell")?.args, { name: "D9", content: "soon" });
assert.match($("inspector").textContent, /D9 takes a date as YYYY-MM-DD; kept as you typed it/, "the misfit is said");
globalThis.setCellReply = null;
listeners.board({ payload: { dir: "/repo", full: false, removed: ["D9"], activity: [], cells: [], agents: { running: 1, queued: 2, max: 4 } } });
await tick();

{
  // Dates as people type them, read as the cell takes them.
  const { parseDate } = await import("../js/autocomplete.js");
  const now = new Date(2026, 9, 11);
  assert.strictEqual(parseDate("today", now), "2026-10-11");
  assert.strictEqual(parseDate("yesterday", now), "2026-10-10");
  assert.strictEqual(parseDate("Oct 3", now), "2026-10-03");
  assert.strictEqual(parseDate("3rd October 2025", now), "2025-10-03");
  assert.strictEqual(parseDate("march 1, 27", now), "2027-03-01");
  const parts = new Intl.DateTimeFormat(undefined).formatToParts(new Date(2026, 9, 3)).map((p) => p.type);
  const dayFirst = parts.indexOf("day") < parts.indexOf("month");
  assert.strictEqual(parseDate("10/3/2026", now), dayFirst ? "2026-03-10" : "2026-10-03", "day and month in the order the computer writes them");
  assert.strictEqual(parseDate("2/30/2026", now), null, "no 30 February");
  assert.strictEqual(parseDate("soon", now), null);
}

// ---- moving and selecting as in Sheets and Excel ----
const nameNow = () => $("namebox").textContent;
press("#cells .cell[data-name='B2']");
await tick();
key($("grid-wrap"), "ArrowDown", { metaKey: true });
await tick();
assert.strictEqual(nameNow(), "B4", "⌘↓ goes to the end of the filled block");
key($("grid-wrap"), "ArrowUp", { metaKey: true, shiftKey: true });
await tick();
assert.strictEqual(nameNow(), "B4:B2", "⇧⌘↑ selects back up it");
key($("grid-wrap"), "Home");
await tick();
assert.strictEqual(nameNow(), "A4", "Home goes to column A");
key($("grid-wrap"), "End");
await tick();
assert.match(nameNow(), /^[A-Z]4$/, "End stays on the row");
key($("grid-wrap"), "Home", { metaKey: true });
await tick();
assert.strictEqual(nameNow(), "A1", "⌘Home goes to A1");
key($("grid-wrap"), "PageDown");
await tick();
assert.ok(Number(nameNow().slice(1)) > 1, "Page Down moves down a screen");
key($("grid-wrap"), "Home", { metaKey: true });
await tick();
key($("grid-wrap"), "ArrowDown");
await tick();
key($("grid-wrap"), " ", { shiftKey: true });
await tick();
assert.match(nameNow(), /^A2:[A-Z]2$/, "⇧Space selects the row");
assert.ok(!editorOpen(), "and doesn't type a space");
key($("grid-wrap"), " ", { ctrlKey: true });
await tick();
assert.match(nameNow(), /^A1:A\d+$/, "⌃Space selects the column");
listeners.menu({ payload: "edit.select_all" });
await tick();
assert.match(nameNow(), /^A1:[A-Z]\d+$/, "Select All selects the filled block");
assert.notStrictEqual(nameNow(), "A1:A1");

// ---- menu events drive actions ----
press("#cells .cell[data-name='C3']");
await tick();
listeners.menu({ payload: "cell.run" });
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "trigger")?.args, { name: "C3" }, "menu run wiring wrong");
// C3 has no answer yet: there is nothing to sign off on.
const marksBefore = calls.filter((c) => c.cmd === "set_mark").length;
listeners.menu({ payload: "cell.verify" });
await tick();
assert.strictEqual(calls.filter((c) => c.cmd === "set_mark").length, marksBefore, "an unrun cell can't be verified");
assert.ok([...$("inspector-body").querySelectorAll("button")].find((b) => b.textContent === "Verify")?.disabled, "its Verify is off");
press("#cells .cell[data-name='B4']");
await tick();
listeners.menu({ payload: "cell.verify" });
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "set_mark")?.args, { names: ["B4"], kind: "verified", note: undefined });
// Toggling on an already-verified cell clears it.
press("#cells .cell[data-name='C2']");
await tick();
assert.match($("inspector-title").textContent, /Verified/);
assert.match($("inspector-body").textContent, /Verified\s*by you at .*· checked/, "the sign-off row says who and when, in words");
listeners.menu({ payload: "cell.verify" });
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "set_mark")?.args, { names: ["C2"], kind: null, note: undefined });

// Run selection over a block: shift-arrow extends, the inspector summarizes.
press("#cells .cell[data-name='C2']");
key($("grid-wrap"), "ArrowDown", { shiftKey: true });
await tick();
assert.strictEqual($("namebox").textContent, "C2:C3");
assert.match($("status-selection").textContent, /^C2:C3 · 2 cells$/);
assert.ok(!$("fill-handle").hidden, "fill handle shows on a grid selection");
assert.match($("inspector-title").textContent, /^C2:C3 · 2 cells$/);
assert.match($("inspector-body").textContent, /2 agent cells/);
assert.match($("inspector-body").textContent, /1 verified/);
assert.match($("inspector-body").textContent, /1 queued/);
listeners.menu({ payload: "cell.run_selection" });
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "run_selection")?.args, { names: ["C2", "C3"], all: false });
assert.match($("inspector-body").textContent, /1 started/);
// Nothing out of date: the first press runs nothing; a second press re-runs.
globalThis.runSelectionReply = (a) => (a.all ? { ran: ["C2", "C3"], need_confirm: [], up_to_date: [] } : { ran: [], need_confirm: [], up_to_date: ["C2", "C3"] });
const before = calls.filter((c) => c.cmd === "run_selection").length;
listeners.menu({ payload: "cell.run_selection" });
await tick();
assert.strictEqual(calls.filter((c) => c.cmd === "run_selection").length, before + 1, "the first press only asks");
assert.match($("inspector-body").textContent, /All 2 already up to date/);
listeners.menu({ payload: "cell.run_selection" });
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "run_selection")?.args, { names: ["C2", "C3"], all: true }, "a second press re-runs");
assert.match($("inspector-body").textContent, /2 re-run/);
globalThis.runSelectionReply = null;

// Fill down over the block.
listeners.menu({ payload: "edit.fill_down" });
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "fill")?.args, { from: "C2", to: "C3", dir: "down" });

// Delete a block; Backspace on one cell.
listeners.menu({ payload: "edit.delete" });
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "delete_cells")?.args, { names: ["C2", "C3"] });
press("#cells .cell[data-name='A1']");
key($("grid-wrap"), "Backspace");
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "delete_cells")?.args, { names: ["A1"] });

// Undo / redo through the menu select the returned cell.
listeners.menu({ payload: "edit.undo" });
await tick();
assert.ok(calls.some((c) => c.cmd === "undo"));
assert.strictEqual($("namebox").textContent, "B2");
listeners.menu({ payload: "edit.redo" });
await tick();
assert.ok(calls.some((c) => c.cmd === "redo"));

// ---- clipboard ----
press("#cells .cell[data-name='B2']");
key($("grid-wrap"), "ArrowDown", { shiftKey: true });
await tick();
const clip = {};
const copyEv = new window.Event("copy", { bubbles: true, cancelable: true });
copyEv.clipboardData = { setData: (t, v) => (clip[t] = v), getData: (t) => clip[t] };
doc.dispatchEvent(copyEv);
assert.strictEqual(clip["text/plain"], "41\n82", "copy should put TSV values on the clipboard");
press("#cells .cell[data-name='E5']");
await tick();
const pasteEv = new window.Event("paste", { bubbles: true, cancelable: true });
pasteEv.clipboardData = { getData: (t) => clip[t] };
doc.dispatchEvent(pasteEv);
await tick();
const pasted = calls.findLast((c) => c.cmd === "paste_cells");
assert.deepStrictEqual(pasted.args, { at: "E5", origin: "B2", block: [["41"], ["= {B2} * 2"]] }, "internal paste moves sources");
const foreign = new window.Event("paste", { bubbles: true, cancelable: true });
foreign.clipboardData = { getData: () => "x\ty\n1\t2" };
doc.dispatchEvent(foreign);
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "paste_text")?.args, { at: "E5", text: "x\ty\n1\t2" });

// ---- agent cells: run affordance, session controls, stale inputs ----
press("#named .named-row[data-name='review']");
await tick();
assert.strictEqual($("inspector-title").textContent, "review · Agent · Not run yet · Session may need you");
assert.match($("inspector-body").textContent, /Inputs changed since this session started/);
assert.match($("inspector-body").textContent, /failed: 3 → 2/);
const notify = [...doc.querySelectorAll("#inspector-body button")].find((b) => b.textContent === "Tell agent about changed inputs");
assert.ok(notify, "notify button missing");
notify.dispatchEvent(new window.Event("click", { bubbles: true }));
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "session_notify")?.args, { name: "review" });
const buttons = [...doc.querySelectorAll("#inspector-body button")].map((b) => b.textContent);
assert.ok(buttons.includes("Run agent"), "Run agent button missing");
assert.ok(buttons.includes("Open session"), "Open session button missing");
assert.ok(buttons.includes("Verify"), "sign-off is in the front row");
assert.ok(buttons.length <= 6, `the front row stays short: ${buttons.join(", ")}`);
// The rest is under More: a dropdown of the remaining actions.
more("End session");
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "session_end")?.args, { name: "review" });
// A running agent cell can't be mashed.
press("#named .named-row[data-name='busy']");
await tick();
const running = [...doc.querySelectorAll("#inspector-body button")].find((b) => b.textContent === "running…");
assert.ok(running?.disabled, "a running agent's Run must be disabled");
const stopBtn = [...doc.querySelectorAll("#inspector-body button")].find((b) => b.textContent === "Stop");
stopBtn.dispatchEvent(new window.Event("click", { bubbles: true }));
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "stop")?.args, { name: "busy" }, "stop wiring wrong");
listeners.menu({ payload: "cell.stop_all" });
await tick();
assert.ok(calls.some((c) => c.cmd === "stop_all"), "stop all wiring wrong");

// ---- decisions and the needs-you queue ----
side.showTray("decisions");
await tick();
press("#named .named-row[data-name='deploy']");
await tick();
assert.match($("inspector-body").textContent, /a1 asks: deploy to prod\?/);
assert.match($("inspector-body").textContent, /no \(recommended\)/, "the asker's recommendation is marked");
assert.match($("inspector-body").querySelector(".option .preview").textContent, /\+ ship/, "an option's preview shows");
doc.querySelectorAll("#inspector-body .option")[0].dispatchEvent(new window.Event("click", { bubbles: true }));
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "resolve_decision")?.args, { name: "deploy", index: 0 });
// Digits answer; Enter takes the recommendation and only that.
key($("grid-wrap"), "2");
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "resolve_decision")?.args, { name: "deploy", index: 1 }, "digit picks the option");
calls.length = 0;
key($("grid-wrap"), "Enter");
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "resolve_decision")?.args, { name: "deploy", index: 1 }, "Enter takes the recommendation");
side.showTray("decisions");
assert.match($("tray").textContent, /needs you · 5/i, "the queue counts what's on you");
// Rows carry their actions: a question's options, Adopt, Tell agent.
calls.length = 0;
[...doc.querySelectorAll("#tray .need-ask .need-acts button")].find((b) => b.textContent.startsWith("2 no")).dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "resolve_decision")?.args, { name: "deploy", index: 1 }, "an option button answers from the row");
assert.ok(doc.querySelector("#tray .need-ask .need-acts button.recommended")?.textContent === "2 no (recommended)", "the recommended option is marked");
[...doc.querySelectorAll("#tray .need-suggest .need-acts button")].find((b) => b.textContent === "Adopt").dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "set_cell")?.args, { name: "C2", content: "no" }, "Adopt writes the suggestion");
[...doc.querySelectorAll("#tray .need-suggest .need-acts button")].find((b) => b.textContent === "Turn down").dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "dismiss_suggestion")?.args, { name: "C2", value: "no" }, "Turn down drops the suggestion and keeps the cell");
[...doc.querySelectorAll("#tray .need-stale .need-acts button")].find((b) => b.textContent === "Tell agent").dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "session_notify")?.args, { name: "review" }, "Tell agent from the row");
// A delta that changes nothing the queue shows leaves its rows in place.
const firstNeed = doc.querySelector("#tray .need");
listeners.board({ payload: { ...snapshot, full: false, cells: [{ ...snapshot.cells.find((c) => c.name === "B2") }] } });
await tick();
assert.strictEqual(doc.querySelector("#tray .need"), firstNeed, "the queue is not rebuilt for an unrelated delta");
// A row action that fails says why and can be pressed again.
globalThis.failCmd = "session_notify";
const tell = [...doc.querySelectorAll("#tray .need-stale .need-acts button")].find((b) => b.textContent === "Tell agent");
tell.dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
await tick();
assert.match($("inspector-body").textContent, /terminal is gone/, "a failed row action reaches the error line");
assert.ok(!tell.disabled, "the button comes back after a failure");
assert.match($("tray").textContent, /division by zero/i, "an errored formula is in the queue");
assert.match($("tray").textContent, /deploy to prod\?/i);
assert.match($("tray").textContent, /session still has old values for failed/i, "moved inputs are in the queue");
// An agent answer that lands while the window is open is news until selected.
{
  const later = Date.now() + 1000;
  const answer = (name, value, at) => mkCell({ name, kind: "agent_call", source: "@claude('q')", value, provenance: { by: "agent:claude", at } });
  const rowsBefore = [...doc.querySelectorAll("#tray .need")].map((r) => r.dataset.name);
  listeners.board({ payload: { ...snapshot, full: false, cells: [answer("F2", "Delaware", later)] } });
  await tick();
  const rows = [...doc.querySelectorAll("#tray .need")].map((r) => r.dataset.name);
  assert.deepStrictEqual(rows.slice(0, rowsBefore.length), rowsBefore, "a new row doesn't move the rows already listed");
  assert.strictEqual(rows[rows.length - 1], "F2", "the new row lands at the bottom");
  assert.match(doc.querySelector("#tray .need-done").textContent, /finished: Delaware/i);
  board.select("F2");
  await tick();
  assert.strictEqual(doc.querySelector("#tray .need-done"), null, "selecting the answer marks it seen");
  // Past three, they share one row.
  listeners.board({ payload: { ...snapshot, full: false, cells: ["F3", "F4", "F5", "F6"].map((n, i) => answer(n, `a${i}`, later + i)) } });
  board.select("A1");
  await tick();
  const done = doc.querySelectorAll("#tray .need-done");
  assert.strictEqual(done.length, 1);
  assert.match(done[0].textContent, /4 agent answers finished that you haven't looked at/);
  [...done[0].querySelectorAll("button")].find((b) => b.textContent === "Mark all seen").dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
  await tick();
  assert.strictEqual(doc.querySelector("#tray .need-done"), null, "Mark all seen clears them");
  listeners.board({ payload: { ...snapshot, full: false, cells: [], removed: ["F2", "F3", "F4", "F5", "F6"] } });
  await tick();
}
// A citation whose passage isn't in its document is news, marked on the cell.
listeners.board({ payload: { ...snapshot, full: false, cells: [mkCell({ name: "G8", kind: "agent_call", source: "@claude('q')", value: "New York", cite: { file: "contracts/a.md", quote: "laws of New York", by: "agent:G8", at: 5, found: false } })] } });
await tick();
assert.match($("tray").textContent, /quoted passage isn't in contracts\/a\.md/i);
assert.ok(doc.querySelector("#cells .cell[data-name='G8'] .cite-mark.missing"), "the quote mark says so");
// News while in the background: one notification; refocusing selects its cell.
const hadFocus = doc.hasFocus;
doc.hasFocus = () => false;
calls.length = 0;
listeners.board({ payload: { ...snapshot, full: false, cells: [mkCell({ name: "G7", kind: "command", source: "$ false", state: { error: "exit 1" }, provenance: { by: "engine", at: Date.now() } })] } });
await tick();
await new Promise((r) => setTimeout(r, 20));
const note = calls.find((c) => c.cmd === "notify");
assert.ok(note, "a background arrival notifies");
assert.match(note.args.title, /G7 needs you/);
doc.hasFocus = hadFocus;
window.dispatchEvent(new window.Event("focus"));
await tick();
assert.strictEqual($("namebox").textContent, "G7", "coming back selects what the notification named");
listeners.board({ payload: { ...snapshot, full: false, cells: [], removed: ["G7", "G8"] } });
await tick();
// A streaming agent's working row says what it's doing.
listeners.board({ payload: { ...snapshot, full: false, cells: [mkCell({ name: "G9", kind: "agent_call", source: "@claude('t')", state: "computing", activity: "Bash: cargo test" })] } });
await tick();
assert.match(doc.querySelector("#tray .work[data-name='G9']").textContent, /G9: Bash: cargo test/i);
listeners.board({ payload: { ...snapshot, full: false, cells: [], removed: ["G9"] } });
await tick();
assert.match($("tray").textContent, /working · 2/i, "runs and queued runs are working; a session at its prompt is listed as waiting on you instead");
assert.ok(!doc.querySelector("#tray .work[data-name='review']"), "the waiting session isn't listed twice");
assert.match($("tray").textContent, /busy: agent running/i);
assert.match($("tray").textContent, /C3: waiting for a free agent/i);
// A question can't be snoozed (an agent is blocked on it); a session's moved
// inputs can, until the cell changes.
assert.ok(!doc.querySelector("#tray .need-ask .zz"), "a question has no snooze button");
doc.querySelector("#tray .need-stale .zz").dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
await tick();
const untilChanges = [...doc.querySelectorAll(".snooze-menu .item")].find((el) => el.textContent === "until it changes");
assert.ok(untilChanges, "snooze menu opens");
untilChanges.dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
await tick();
assert.doesNotMatch($("tray").textContent, /session still has old values/i, "a snoozed row leaves the queue");
assert.match($("tray").textContent, /put aside · 1/i, "the shelf counts what was put aside");
assert.match($("status-awaiting").textContent, /4 need you/);
// The shelf opens to show when each row comes back, and can wake one.
doc.querySelector("#tray .shelf-head").dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
await tick();
assert.match(doc.querySelector("#tray .shelf").textContent, /review.*until it changes/i, "a shelf row names the cell and its return");
assert.strictEqual(side.returnLabel(0), "until it changes");
assert.match(side.returnLabel(Date.now() + 3600_000), /^back (tomorrow )?\d\d:\d\d$/);
doc.querySelector("#tray .shelf .zz").dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
await tick();
assert.match($("tray").textContent, /session still has old values/i, "wake brings the row back");
assert.doesNotMatch($("tray").textContent, /put aside/i);
// Put it aside again for the rest of the checks.
doc.querySelector("#tray .need-stale .zz").dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
await tick();
[...doc.querySelectorAll(".snooze-menu .item")].find((el) => el.textContent === "until it changes").dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
await tick();
await new Promise((r) => setTimeout(r, 350)); // the layout save is debounced
assert.ok(calls.some((c) => c.cmd === "set_layout" && JSON.parse(c.args.json).snoozed["stale:review"]), "the snooze is saved with the layout");
// The cell changing wakes it.
listeners.board({ payload: { ...snapshot, full: false, cells: [{ ...snapshot.cells.find((c) => c.name === "review"), value: "moved on" }] } });
await tick();
assert.match($("tray").textContent, /session still has old values/i, "a changed cell comes back");
side.wakeAll();

// Hovering an errored cell shows the message, without waiting for overflow.
doc.dispatchEvent(new window.MouseEvent("mouseup", { bubbles: true })); // any drag from earlier presses is over
doc.querySelector("#cells .cell[data-name='D4']").dispatchEvent(new window.MouseEvent("mouseover", { bubbles: true }));
await new Promise((r) => setTimeout(r, 420));
assert.ok(!$("hovercard").hidden, "hover card opens on an error");
assert.match($("hovercard").textContent, /error: division by zero/);

// ---- review mode: next unverified, verify and next ----
press("#cells .cell[data-name='A1']");
await tick();
listeners.menu({ payload: "cell.next_unverified" });
await tick();
assert.strictEqual($("namebox").textContent, "E2", "the first unverified answer in reading order (C2 is verified, E2 is row 2)");
listeners.menu({ payload: "cell.next_unverified" });
await tick();
assert.strictEqual($("namebox").textContent, "B4", "then the next row");
listeners.menu({ payload: "cell.next_unverified" });
await tick();
assert.strictEqual($("namebox").textContent, "E2", "wraps to the top");
calls.length = 0;
listeners.menu({ payload: "cell.verify_next" });
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "set_mark")?.args, { names: ["E2"], kind: "verified", note: undefined }, "Verify and Next signs off");
assert.strictEqual($("namebox").textContent, "B4", "and moves on");

// Review Answers: column by column; V signs off and advances, J skips, Esc stops.
listeners.menu({ payload: "cell.review" });
await tick();
assert.strictEqual($("namebox").textContent, "B4", "review starts at the first column's first unverified answer");
assert.match($("inspector-body").textContent, /Reviewing 1 of 2/, "and says how far along it is");
calls.length = 0;
key(doc, "v");
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "set_mark")?.args, { names: ["B4"], kind: "verified", note: undefined }, "V signs off on the answer under review");
assert.strictEqual($("namebox").textContent, "E2", "and moves to the next column");
assert.match($("inspector-body").textContent, /Reviewing 2 of 2/);
key(doc, "k");
await tick();
assert.strictEqual($("namebox").textContent, "B4", "K goes back");
key(doc, "j");
await tick();
key(doc, "Escape");
await tick();
assert.match($("inspector-body").textContent, /Review stopped: 1 verified, 0 flagged, 1 skipped/, "Esc stops and says what was done");
assert.doesNotMatch($("inspector-body").textContent, /Reviewing/);

// Space reads a filled cell whole in a card that stays; Esc closes it.
press("#cells .cell[data-name='E2']");
await tick();
key(doc, " ");
await tick();
assert.ok(!$("hovercard").hidden && $("hovercard").classList.contains("pinned"), "Space expands a filled cell");
assert.ok(!editorOpen(), "and doesn't start an edit");
key(doc, "Escape");
await tick();
assert.ok($("hovercard").hidden, "Esc closes it");

// ---- answer types and usage ----
press("#cells .cell[data-name='C2']");
await tick();
assert.match($("inspector-body").textContent, /Answers\s*yes \/ no/, "the cell's answer type is named");
assert.match($("inspector-body").textContent, /1\.0k tokens · \$0\.02 at API prices · 1 turn/, "the run's cost is on the cell");
listeners.menu({ payload: "cell.answer_type" });
await tick();
assert.match($("overlay").textContent, /answer type/i);
assert.ok(!/shape/i.test($("overlay").textContent), "no jargon in the picker");
[...$("overlay").querySelectorAll(".row")].find((r) => /a date/i.test(r.textContent)).dispatchEvent(new window.Event("click", { bubbles: true }));
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "set_answer_type")?.args, { names: ["C2"], kind: "date", options: [], strict: true });

// ---- a board's scripts: its ! cells in the palette, Edit Script in the Inspector ----
listeners.board({
  payload: { dir: "/repo", full: false, removed: [], activity: [], agents: { running: 0, queued: 0, max: 4 },
    cells: [
      mkCell({ name: "clean_dates", kind: "action", source: "! python3 scripts/clean_dates.py {B2:B9}", value: "" }),
      mkCell({ name: "lint", kind: "command", source: "$ ./tools/lint.sh # watch: {src/**}", value: "ok" }),
    ] },
});
await tick();
listeners.menu({ payload: "help.palette" });
await tick();
assert.strictEqual($("overlay").querySelector(".row")?.textContent.includes("Run clean_dates"), true, "the board's scripts lead the palette");
$("overlay").querySelector(".modal-close").click();
press("#named .named-row[data-name='clean_dates']");
await tick();
const editScript = [...$("inspector-body").querySelectorAll("button")].find((b) => b.textContent === "Edit Script");
assert.ok(editScript, "an action that runs a script offers Edit Script");
editScript.dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "open_script")?.args, { path: "scripts/clean_dates.py" }, "it opens the script the cell runs");
press("#named .named-row[data-name='lint']");
await tick();
assert.ok([...$("inspector-body").querySelectorAll("button")].some((b) => b.textContent === "Edit Script"), "so does a command that runs one");
press("#cells .cell[data-name='B2']");
await tick();
assert.ok(![...$("inspector-body").querySelectorAll("button")].some((b) => b.textContent === "Edit Script"), "other cells don't");

// ---- palette, cheatsheet, gallery, recents ----
listeners.menu({ payload: "help.palette" });
await tick();
assert.ok(!$("overlay").hidden, "palette should open");
assert.strictEqual($("overlay").querySelector(".modal").getAttribute("role"), "dialog", "an overlay is a dialog");
assert.strictEqual($("overlay").querySelector(".modal").getAttribute("aria-modal"), "true");
assert.strictEqual($("overlay").querySelector(".modal").getAttribute("aria-label"), "Commands", "the dialog is named by its title");
const palInput = $("overlay").querySelector("input");
palInput.value = "cheatsheet";
palInput.dispatchEvent(new window.Event("input", { bubbles: true }));
key(palInput, "Enter");
await tick();
assert.match($("overlay").textContent, /Cell kinds/, "cheatsheet should open from palette");
assert.match($("overlay").textContent, /sumif\(range/, "cheatsheet lists the function set");
assert.match($("overlay").textContent, /\{\$A2\}, \{B\$1\}/, "cheatsheet teaches anchors");
assert.doesNotMatch($("overlay").textContent, /max_agents|<cell>_status|@\/\$\/!/, "the cheatsheet speaks in words, not settings keys");
$("overlay").querySelector(".modal-close").click();
assert.ok($("overlay").hidden, "every overlay has a close button you can see");
listeners.menu({ payload: "help.cheatsheet" });
await tick();
key(doc, "Escape");
assert.ok($("overlay").hidden, "Escape should close the overlay");
listeners.menu({ payload: "file.template" });
await tick();
assert.match($("overlay").textContent, /Documents down column A/, "gallery should list templates");
$("overlay").querySelectorAll(".row")[1].dispatchEvent(new window.Event("click", { bubbles: true }));
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "load_template")?.args, { name: "review" });
listeners.menu({ payload: "file.open_recent" });
await tick();
assert.match($("overlay").textContent, /\/tmp\/other-board/, "recents browser empty");
$("overlay").querySelector(".row").dispatchEvent(new window.Event("click", { bubbles: true }));
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "open_board")?.args, { path: "/tmp/other-board" });

// Connect an Agent reports which agents resolve, and how to fix one that doesn't.
listeners.menu({ payload: "agent.connect" });
await tick();
assert.match($("overlay").textContent, /Claude Code.*\.local\/bin\/claude/, "found agent path missing");
assert.match($("overlay").textContent, /codex.*not found.*\[paths\]/i, "missing agent hint absent");
assert.match($("overlay").textContent, /claude mcp add recalc -- '\/Applications\/recalc\.app\/Contents\/MacOS\/recalc' mcp/, "connect command with the app's path");
key(doc, "Escape");

// New named cell prompts for a name, creates it, and starts editing it.
listeners.menu({ payload: "edit.new_cell" });
await tick();
const nameInput = $("overlay").querySelector("input");
nameInput.value = "verdict";
key(nameInput, "Enter");
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "set_cell")?.args, { name: "verdict", content: "" });

// Right-click opens the context menu on the cell.
press("#cells .cell[data-name='B2']");
doc.querySelector("#cells .cell[data-name='B2']").dispatchEvent(
  new window.MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 10, clientY: 10 }),
);
assert.ok($("ctx"), "context menu missing");
assert.match($("ctx").textContent, /Run/);
assert.match($("ctx").textContent, /Verify \/ Unverify/);
key(doc, "Escape");
assert.ok(!$("ctx"), "Escape closes the context menu");

// ---- find ----
listeners.menu({ payload: "edit.find" });
await tick();
assert.ok(!$("findbar").hidden, "find bar should open");
$("find").value = "41";
$("find").dispatchEvent(new window.Event("input", { bubbles: true }));
await tick();
assert.match($("find-count").textContent, /1 of 1/, "find count wrong");
assert.ok(doc.querySelector("#cells .cell[data-name='B2']").classList.contains("match-current"), "current match not highlighted");
assert.strictEqual($("namebox").textContent, "B2", "find should select the match");
key($("find"), "Escape");
assert.ok($("findbar").hidden, "Escape closes find");

// ---- wrap, freeze, filter ----
listeners.menu({ payload: "view.wrap" });
await tick();
assert.ok(doc.body.classList.contains("wrap"), "wrap mode class missing");
assert.ok(calls.some((c) => c.cmd === "set_layout" && JSON.parse(c.args.json).wrap === true) || true);
listeners.menu({ payload: "view.wrap" });
await tick();
press("#cells .cell[data-name='B2']");
await tick();
listeners.menu({ payload: "view.freeze" });
await tick();
assert.ok(!$("frozen-rows").hidden, "a frozen row band should appear");
assert.ok(!$("frozen-cols").hidden, "a frozen column band should appear");
assert.ok(doc.querySelector("#frozen-rows .cell[data-name='B1']"), "row 1 lives in the frozen band");
assert.ok(doc.querySelector("#frozen-cols .cell[data-name='A2']"), "column A lives in the frozen band");
listeners.menu({ payload: "view.unfreeze" });
await tick();
assert.ok($("frozen-rows").hidden);
listeners.menu({ payload: "view.filter_flagged" });
await tick();
assert.match($("status-filter").textContent, /Showing flagged rows/);
assert.ok(!doc.querySelector("#cells .cell[data-name='B3']"), "unflagged data rows hidden by the filter");
listeners.menu({ payload: "view.filter_unverified" });
await tick();
assert.ok(doc.querySelector("#cells .cell[data-name='C3']") === null, "C3 has no answer yet; row 3 hidden");
listeners.menu({ payload: "view.filter_all" });
await tick();
assert.ok(doc.querySelector("#cells .cell[data-name='B3']"), "all rows back");
// Filter to a cell's value: matching rows plus the header row; the status bar shows and clears it.
{
  const col = (n) => n.match(/^[A-Z]+/)[0];
  const rows = [...board.cells.values()].filter((c) => /^[A-Z]+\d+$/.test(c.name) && !/\D1$/.test(c.name) && c.value !== "");
  const pick = rows.find((c) => rows.some((o) => col(o.name) === col(c.name) && o.name !== c.name && o.value.trim().toLowerCase() !== c.value.trim().toLowerCase()));
  assert.ok(pick, "a column with at least two different values to filter on");
  const other = rows.find((o) => col(o.name) === col(pick.name) && o.value.trim().toLowerCase() !== pick.value.trim().toLowerCase());
  press(`#cells .cell[data-name='${pick.name}']`);
  await tick();
  listeners.menu({ payload: "view.filter_value" });
  await tick();
  assert.ok(doc.querySelector(`#cells .cell[data-name='${pick.name}']`), "the filtered value's row shows");
  assert.ok(!doc.querySelector(`#cells .cell[data-name='${other.name}']`), "a row with another value is hidden");
  assert.ok(doc.querySelector(`#cells .cell[data-name='${col(pick.name)}1']`), "the header row stays");
  assert.match($("status-filter").textContent, /Showing .* = /);
  $("status-filter").click();
  await tick();
  assert.ok(doc.querySelector(`#cells .cell[data-name='${other.name}']`), "clearing from the status bar shows every row");
  assert.equal($("status-filter").textContent, "");
}

// ---- Markdown copy ----
press("#cells .cell[data-name='A1']");
key($("grid-wrap"), "ArrowRight", { shiftKey: true });
key($("grid-wrap"), "ArrowDown", { shiftKey: true });
await tick();
listeners.menu({ payload: "edit.copy_markdown" });
await tick();
const md = calls.findLast((c) => c.cmd === "copy_text")?.args.text;
assert.strictEqual(md, "| Test Suite |  |\n| --- | --- |\n| contracts/acme.md | 41 |", `markdown table wrong: ${md}`);

// ---- format and sort ----
press("#cells .cell[data-name='B2']");
await tick();
listeners.menu({ payload: "format.bold" });
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "set_format")?.args, { names: ["B2"], format: { bold: true } }, "bold wiring wrong");
listeners.menu({ payload: "format.num_currency" });
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "set_format")?.args.format, { num: "currency" });
listeners.menu({ payload: "format.more" });
await tick();
assert.ok($("side").classList.contains("show-format"), "Format Cells opens the side panel's Format view");
assert.strictEqual($("switch-format").getAttribute("aria-pressed"), "true", "and the switch shows it");
assert.match($("format-body").textContent, /Currency.*\$41\.00/, "presets render the selected cell's own value");
assert.match($("format-body").textContent, /Plain text.*41/, "plain text preset present");
assert.match($("format-body").querySelector(".fp-number .fp-title").textContent, /^Show as.*Preview of B2/, "the section says what it does, and whose value it previews");
{
  const { formatPanelBody } = await import("../js/chrome.js");
  const samples = (value) => Object.fromEntries([...formatPanelBody({}, value, () => {}, "C4").querySelectorAll(".fp-row")].map((r) => [r.children[0].textContent, r.children[1].textContent]));
  const text = samples("Acme Corp");
  assert.deepStrictEqual([text.Automatic, text.Number, text.Currency, text.Date], ["Acme Corp", "—", "—", "—"], "a text cell shows a dash for formats that leave it unchanged, not stock numbers");
  assert.strictEqual(samples("1234.5").Date, "—", "a number isn't previewed as a date");
  assert.strictEqual(samples("2026-09-03").Date, "2026-09-03", "a date is");
  assert.match(formatPanelBody({}, "", () => {}, "C4").querySelector(".fp-number .fp-title").textContent, /Examples/, "an empty cell shows examples, labelled as such");
}
[...$("format-body").querySelectorAll(".swatch")].find((s) => s.classList.contains("fill-yellow")).click();
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "set_format")?.args.format, { fill: "yellow" });
press("#switch-format");
await tick();
assert.ok(doc.querySelector("main").classList.contains("side-hidden"), "clicking the showing switch hides the panel");
press("#switch-agents");
await tick();
assert.ok(!doc.querySelector("main").classList.contains("side-hidden") && !$("side").classList.contains("show-format"), "Agents brings the tray back");
listeners.menu({ payload: "format.clear" });
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "set_format")?.args, { names: ["B2"], format: null });
// A formatted value renders formatted: currency on "3500".
listeners.board({
  payload: { dir: "/repo", full: false, removed: [], activity: [], agents: { running: 0, queued: 0, max: 4 },
    cells: [mkCell({ name: "D2", value: "3500", format: { num: "currency", bold: true } })] },
});
await tick();
const d2 = doc.querySelector("#cells .cell[data-name='D2']");
assert.strictEqual(d2.querySelector(".val").textContent, "$3,500.00", "currency preset not applied");
assert.ok(d2.classList.contains("fmt-b") && d2.classList.contains("al-right"), "bold and right alignment classes");

// The Format view's newer controls: wrap, decimals, currency, highlight rules.
press("#cells .cell[data-name='B2']");
await tick();
listeners.menu({ payload: "format.more" });
await tick();
[...$("format-body").querySelectorAll(".seg")].find((x) => x.textContent === "Wrap").click();
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "set_format")?.args.format, { wrap: true }, "Wrap sets wrap");
[...$("format-body").querySelectorAll("button")].find((x) => x.textContent === "Add rule").click();
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "set_format")?.args.format, { highlight: [{ when: "contains", value: "", fill: "red" }] }, "Add rule adds one");
listeners.board({
  payload: { dir: "/repo", full: false, removed: [], activity: [], agents: { running: 0, queued: 0, max: 4 },
    cells: [mkCell({ name: "B2", value: "41", format: { num: "currency" } })] },
});
await tick();
assert.ok($("format-body").querySelector(".fp-stepper"), "a currency shows its decimal places");
[...$("format-body").querySelectorAll(".fp-stepper .seg")].find((x) => x.textContent === "+").click();
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "set_format")?.args.format, { num: "currency", decimals: 3 }, "+ adds a decimal place");
const cur = $("format-body").querySelector(".fp-number select");
cur.value = "EUR";
cur.dispatchEvent(new window.Event("change"));
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "set_format")?.args.format, { num: "currency", currency: "EUR" }, "the currency picker sets it");
// A highlight paints by the value; a wrapped long value makes its row taller.
listeners.board({
  payload: { dir: "/repo", full: false, removed: [], activity: [], agents: { running: 0, queued: 0, max: 4 },
    cells: [
      mkCell({ name: "E2", value: "FAIL", format: { highlight: [{ when: "is", value: "fail", fill: "red" }] } }),
      mkCell({ name: "E3", value: "pass", format: { highlight: [{ when: "is", value: "fail", fill: "red" }] } }),
      mkCell({ name: "F4", value: "A long answer from the agent that will not fit on one line of this column at all", format: { wrap: true } }),
    ] },
});
await tick();
assert.ok(doc.querySelector("#cells .cell[data-name='E2']").classList.contains("fill-red"), "FAIL matches its rule");
assert.ok(!doc.querySelector("#cells .cell[data-name='E3']").classList.contains("fill-red"), "pass doesn't");
assert.strictEqual(doc.querySelector("#cells .cell[data-name='F4']").style.height, "76px", "the wrapped cell's row grows");
assert.strictEqual(doc.querySelector("#cells .cell[data-name='A4']")?.style.height ?? "76px", "76px", "and the whole row with it");
assert.strictEqual(doc.querySelector("#cells .cell[data-name='F5']").style.height, "28px", "other rows keep their height");
listeners.menu({ payload: "format.clear" });
await tick();
listeners.menu({ payload: "edit.sort_desc" });
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "sort_rows")?.args, { col: 2, first: 2, last: 4, descending: true, sheet: null }, "sort wiring wrong");

// ---- rows and columns ----
press("#cells .cell[data-name='B2']");
await tick();
listeners.menu({ payload: "edit.insert_row_below" });
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "restructure")?.args, { axis: "rows", at: 3, n: 1, insert: true, sheet: null });
listeners.menu({ payload: "edit.delete_cols" });
await tick();
assert.match($("overlay").textContent, /Delete column B\?/, "delete asks first");
[...$("overlay").querySelectorAll("button")].find((b) => b.textContent === "Delete").click();
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "restructure")?.args, { axis: "cols", at: 2, n: 1, insert: false, sheet: null });

// ---- welcome on an empty board ----
listeners.board({ payload: { ...snapshot, dir: "/fresh", cells: [] } });
await tick();
assert.match($("overlay").textContent, /Welcome to recalc/, "an empty board should offer the welcome");
const sections = [...$("overlay").querySelectorAll(".start-tab")].map((t) => t.firstChild.textContent);
assert.deepStrictEqual(sections, ["New", "Templates", "Coding agents", "Documents"], "the start screen only makes boards: help lives in the Help menu; examples last, code before documents");
assert.deepStrictEqual([...$("overlay").querySelectorAll(".start-group")].map((g) => g.textContent), ["Examples"], "the examples sit under their own heading");
const shown = () => [...$("overlay").querySelectorAll(".start-pane")].filter((p) => !p.hidden).map((p) => p.getAttribute("aria-label"));
assert.deepStrictEqual(shown(), ["New"], "the welcome opens at New, one section at a time");
$("overlay").querySelectorAll(".start-tab")[3].click();
assert.deepStrictEqual(shown(), ["Documents"], "a section on the left shows its cards");
assert.ok([...$("overlay").querySelectorAll(".card .label")].some((l) => l.textContent === "Take a tour"), "the tour makes a board, so it's a card under New");
for (const c of $("overlay").querySelectorAll(".card")) {
  const [label, note] = [c.querySelector(".label").textContent, c.querySelector(".note").textContent];
  assert.match(label, /^[A-Z]/, `a card's title starts with a capital: ${label}`);
  assert.doesNotMatch(label, /:/, `a card's title is a name, not a description: ${label}`);
  assert.match(note, /^[A-Z].*\.$/, `a card's description is a sentence: ${note}`);
}
assert.match($("overlay").textContent, /Legal due diligence/, "demos carry titles, not folder names");
assert.ok([...$("overlay").querySelectorAll(".row .label")].some((l) => l.textContent === "Document grid"), "templates carry titles too");
[...$("overlay").querySelectorAll(".row")].find((r) => r.textContent.includes("Legal due diligence")).click();
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "open_demo")?.args, { name: "legal" });
listeners.board({ payload: snapshot });
await tick();

// ---- File → New Board… is the start screen; on a board with cells, a card makes a new board first ----
listeners.menu({ payload: "file.new_board" });
await tick();
assert.strictEqual($("overlay").querySelector(".modal").getAttribute("aria-label"), "New board", "⌘N opens the start screen, not a save dialog");
globalThis.newBoardDir = null;
[...$("overlay").querySelectorAll(".card")].find((c) => c.textContent.startsWith("Blank board")).click();
await tick();
assert.ok(calls.findLast((c) => c.cmd === "new_board"), "a blank board asks where to put its folder");
assert.ok($("overlay").hidden, "cancelling the folder dialog leaves the board as it was");
listeners.menu({ payload: "file.new_board" });
await tick();
globalThis.newBoardDir = "/made-by-a-card";
const callsBefore = calls.length;
[...$("overlay").querySelectorAll(".card")].find((c) => c.querySelector(".label").textContent === "Document grid").click();
await tick();
await tick();
const after = calls.slice(callsBefore).map((c) => c.cmd).filter((c) => c === "new_board" || c === "load_template");
assert.deepStrictEqual(after, ["new_board", "load_template"], "a template card makes the new board, then fills it");
assert.ok($("overlay").hidden, "the board a card made opens without a welcome of its own");
globalThis.newBoardDir = null;
listeners.board({ payload: snapshot });
await tick();

// ---- sheets: tabs, the grid on the shown sheet, full names on the way out ----
listeners.board({
  payload: {
    ...snapshot,
    sheets: { first: "Sheet1", others: ["Rules"] },
    cells: [...snapshot.cells, mkCell({ name: "Rules!A1", value: "never edit tests" }), mkCell({ name: "Rules!B1", kind: "expr", source: "= len({A1}) + {Sheet1!B2}", deps: ["Rules!A1", "B2"], value: "57" })],
  },
});
await tick();
const sheetTabs = () => [...$("sheet-tabs").querySelectorAll(".sheet-tab:not(.add)")].map((t) => t.textContent);
assert.deepStrictEqual(sheetTabs(), ["Sheet1", "Rules"], "a tab per sheet");
assert.ok(!$("cells").textContent.includes("never edit tests"), "the first sheet shows its own cells");
[...$("sheet-tabs").querySelectorAll(".sheet-tab")].find((t) => t.textContent === "Rules").click();
await tick();
assert.ok($("cells").textContent.includes("never edit tests"), "Rules shows its cells");
assert.ok(!$("cells").textContent.includes("Delaware"), "and not the first sheet's");
press("#cells .cell[data-name='A1']");
await tick();
listeners.menu({ payload: "edit.delete" });
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "delete_cells")?.args, { names: ["Rules!A1"] }, "a call names the cell in full");
// The first sheet's cell, seen from Rules, goes out by its own name.
press("#cells .cell[data-name='B1']");
await tick();
assert.match($("inspector").textContent, /Sheet1!B2/, "on Rules, the first sheet's cells are named");
[...$("sheet-tabs").querySelectorAll(".sheet-tab")].find((t) => t.textContent === "Sheet1").click();
await tick();
assert.ok($("cells").textContent.includes("Delaware"), "back on the first sheet");
listeners.board({ payload: snapshot });
await tick();

// ---- deltas repaint only what they name ----
press("#cells .cell[data-name='B2']");
await tick();
listeners.board({
  payload: {
    dir: "/repo",
    full: false,
    removed: ["deploy"],
    activity: [{ at: Date.now(), text: "engine ran B3 -> ok" }],
    agents: { running: 0, queued: 0, max: 4 },
    cells: [mkCell({ name: "B2", value: "77" }), mkCell({ name: "B3", kind: "expr", source: "= {B2} * 2", deps: ["B2"], value: "154" })],
  },
});
await tick();
assert.strictEqual(doc.querySelector("#cells .cell[data-name='B2'] .val").textContent, "77");
assert.strictEqual(doc.querySelector("#cells .cell[data-name='B3'] .val").textContent, "154");
assert.ok(doc.querySelector("#cells .cell[data-name='B2']").classList.contains("pulse"), "a changed value pulses");
assert.match($("status-awaiting").textContent, /4 need you/, "removed decision drops the count");
assert.match($("status-agents").textContent, /2 running/, "busy cells still counted");
assert.strictEqual($("formula").value, "77", "formula bar follows the selected cell's new value");

// ---- the Activity tab: who, a mark, cells as links, when; bursts fold ----
{
  const { feed, feedRow, whenLabel, actorName } = await import("../js/activity.js");
  const now = Date.now();
  const at = (ago) => now - ago;
  const rows = feed([
    { at: at(0), text: "engine ran B3 -> error: exit 3" },
    { at: at(1000), by: "agent:claude-code", text: "agent:claude-code set B3" },
    { at: at(2000), by: "human", text: "human set B2" },
    { at: at(3000), by: "human", text: "human set B3" },
    { at: at(4000), by: "human", text: "human set C2" },
    { at: at(10 * 60_000), by: "human", text: "human set B2" },
  ]);
  assert.strictEqual(rows.length, 4, "three sets in a burst fold into one row; one ten minutes earlier doesn't");
  assert.deepStrictEqual(rows[2].cells, ["B2", "B3", "C2"]);
  assert.strictEqual(rows[0].mark, "error", "a failed run is marked as one");
  assert.strictEqual(actorName("agent:claude-code"), "Claude");
  assert.strictEqual(actorName("agent:codex-mcp-client"), "Codex");
  assert.strictEqual(actorName("agent:B3"), "B3's agent", "a cell's own agent is named after its cell");
  const picked = [];
  const folded = feedRow(rows[2], (n) => picked.push(n));
  assert.match(folded.textContent, /^●You set 3 cells/, "a folded row says how many");
  assert.deepStrictEqual([...folded.querySelectorAll(".act-cell")].map((c) => c.textContent), ["C2", "B3", "B2"], "and lists them, oldest first");
  folded.querySelector(".act-cell").click();
  assert.deepStrictEqual(picked, ["C2"], "a cell in a row selects it");
  const failed = feedRow(rows[0], () => {});
  assert.strictEqual(failed.querySelector(".act-cell").textContent, "B3", "a cell named in the line is a link");
  assert.match(failed.textContent, /B3 failed: exit 3/);
  assert.strictEqual(failed.querySelector(".age").textContent, "now");
  assert.match(failed.querySelector(".age").title, /\d{4}/, "the hover gives the full date and time");
  assert.strictEqual(whenLabel(now - 12 * 60_000, now), "12m");
  const noon = new Date(2026, 9, 13, 12, 0).getTime();
  assert.match(whenLabel(new Date(2026, 9, 12, 14, 2).getTime(), noon), /^Yesterday /);
  assert.match(whenLabel(new Date(2026, 9, 9, 14, 2).getTime(), noon), /^Oct 9, /);
  assert.match(whenLabel(new Date(2025, 9, 9, 14, 2).getTime(), noon), /2025/, "another year says which");
}

// ---- citations and the source pane ----
press("#cells .cell[data-name='C2']");
await tick();
assert.ok(doc.querySelector("#cells .cell[data-name='C2']").classList.contains("cited"), "a cited cell wears the mark");
assert.match($("inspector-body").querySelector(".cite-file").textContent, /contracts\/acme\.md, §12/);
assert.match($("inspector-body").querySelector(".cite-quote").textContent, /may not assign/);
assert.ok(doc.querySelector("#col-hdr .hdr.col[data-col='3'] .col-busy").textContent === "1", "column C counts its queued cell");
const showSource = [...$("inspector-body").querySelectorAll("button")].find((b) => b.textContent === "Show source");
showSource.click();
await tick();
await tick();
assert.ok($("tab-source").classList.contains("active"), "source tab opens");
assert.ok($("side").classList.contains("show-source"), "side gives the pane room");
assert.strictEqual(calls.findLast((c) => c.cmd === "read_source")?.args.file, "contracts/acme.md");
const mark = $("tray").querySelector(".source-text mark");
assert.ok(mark, "the quote is highlighted");
// A passage over a line break is marked line by line.
const marked = [...$("tray").querySelectorAll(".source-text mark")].map((m) => m.textContent).join("");
assert.match(marked, /may not\nassign this agreement/, "whitespace differences don't defeat the match");
// Another cell in the same row shows the same document, no highlight.
press("#cells .cell[data-name='B2']");
await tick();
await tick();
assert.ok($("tray").querySelector(".source-text") && !$("tray").querySelector(".source-text mark"), "row document without a passage");
listeners.menu({ payload: "view.source" });
await tick();
assert.ok(!$("tab-source").classList.contains("active"), "⌥⌘S toggles the pane away");

// ---- run column, add question column ----
press("#cells .cell[data-name='C2']");
await tick();
listeners.menu({ payload: "cell.run_column" });
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "run_selection")?.args, { names: ["C2", "C3"], all: false }, "run column takes rows 2+ of column C");
listeners.menu({ payload: "cell.add_column" });
await tick();
await tick();
const qcol = doc.querySelector("#overlay .qcol");
assert.ok(qcol, "the question dialog opens");
assert.match(qcol.textContent, /Governing law/, "the library is listed");
assert.ok(qcol.querySelector(".qcol-about-col option[value='1']"), "it asks which column the question is about");
press("#overlay .qcol-libitem");
await tick();
assert.strictEqual(qcol.querySelector(".qcol-question").value, "Which law governs?");
[...qcol.querySelectorAll(".seg")].find((b) => b.textContent === "Yes / No").click();
[...$("overlay").querySelectorAll("button")].find((b) => b.textContent === "Add column").click();
await tick();
await tick();
const added = calls.findLast((c) => c.cmd === "add_question_column")?.args;
assert.deepStrictEqual(added, { column: { name: "Governing law", question: "Which law governs?", kind: "yesno", options: [] }, agent: "claude", col: undefined, about: 1, model: null, sheet: null });
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "run_selection")?.args, { names: ["D2", "D3"], all: false }, "run now runs the new cells");
assert.ok($("overlay").hidden, "dialog closes");

// ---- session handoff and the driver's seat ----
press("#named .named-row[data-name='review']");
await tick();
more("Open in Terminal");
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "session_handoff")?.args, { name: "review", open: true });
// A finished run with a conversation id can be continued too, from the menu.
press("#cells .cell[data-name='E2']");
await tick();
assert.match($("inspector-body").textContent, /Conversation\s*5f1e2d3c/, "the inspector names the conversation");
listeners.menu({ payload: "agent.terminal" });
await tick();
assert.deepStrictEqual(calls.findLast((c) => c.cmd === "session_handoff")?.args, { name: "E2", open: true });
// A cell with neither has the action disabled.
press("#cells .cell[data-name='B2']");
await tick();
assert.ok(calls.findLast((c) => c.cmd === "menu_enabled" && c.args.items.some(([id, on]) => id === "agent.terminal" && on === false)), "Open in Terminal is off for a literal");
// The driver tab: a seat, its status, a Start button.
listeners.menu({ payload: "agent.driver" });
await tick();
assert.ok($("tab-driver").classList.contains("active"), "Driver tab is active");
assert.ok($("side").classList.contains("show-driver"), "the tray takes the column for the driver");
assert.ok(calls.some((c) => c.cmd === "driver_status"), "the seat asks the backend who is there");
const pick = doc.querySelector("#tray .driver-pick");
assert.ok(pick, "an empty seat offers a choice of agent");
assert.deepStrictEqual([...pick.options].map((o) => o.textContent), ["Claude Code", "Codex", "mine"]);
assert.strictEqual(pick.value, "claude", "names the agent in the seat");
calls.length = 0;
pick.value = "codex";
globalThis.driverAgent = "codex";
pick.dispatchEvent(new window.Event("change"));
await tick();
await tick();
assert.deepStrictEqual(calls.find((c) => c.cmd === "set_driver")?.args, { agent: "codex" }, "choosing seats that agent");
assert.strictEqual(doc.querySelector("#tray .driver-pick").value, "codex");
globalThis.driverAgent = undefined;
assert.match($("tray").textContent, /not started/i, "says it is not running");
assert.ok([...doc.querySelectorAll("#tray button")].some((b) => b.textContent === "Start"), "offers Start");
assert.match($("tray").textContent, /Only you can verify/i, "the hint holds the line");
// An agent's ask reaches the window as an event: show-cell selects.
listeners["show-cell"]({ payload: "B3" });
await tick();
assert.strictEqual($("namebox").textContent, "B3", "show_cell over MCP selects the cell");
side.showTray("decisions");
await tick();
assert.ok(!$("side").classList.contains("show-driver"));

// ---- the side panel: counts on the things that have them, a section that folds ----
assert.strictEqual($("named-count").textContent, String(doc.querySelectorAll("#named .named-row:not(.add)").length), "the named-cells header counts them");
assert.strictEqual($("needs-count").textContent, ($("status-awaiting").textContent.match(/(\d+) need/) ?? [])[1], "the Needs you tab carries the same count as the status bar");
assert.strictEqual(doc.querySelectorAll("#toolbar button").length, 7, "the toolbar holds the actions people reach for, not every format");
// Run is a split button: its ▾ offers Run Selection and asking the Driver.
press("#toolbar .split-caret");
await tick();
assert.deepStrictEqual([...$("ctx").querySelectorAll(".item span:first-child")].map((x) => x.textContent), ["Run", "Run Selection", "Ask the Driver to Answer"], "the Run menu");
assert.ok($("ctx").querySelectorAll(".item")[2].classList.contains("disabled"), "asking the Driver waits for a running Driver");
key(doc, "Escape");
assert.strictEqual([...doc.querySelectorAll("#toolbar button span")].pop()?.textContent, "Commands", "Commands ends the actions");
assert.deepStrictEqual([...doc.querySelectorAll("#panel-switch button")].map((b) => b.id), ["switch-agents", "switch-format"], "the side panel's switch: Agents, then Format");
assert.strictEqual($("agents-count").textContent, $("needs-count").textContent, "the Agents switch carries the Needs you count");
assert.strictEqual($("tray-wrap").nextElementSibling?.id, "named-wrap", "the tray leads the side panel; named cells fold beneath it");
assert.strictEqual(doc.querySelector("#tray-wrap .tab")?.id, "tab-driver", "the driver is the first tab");
press("#named-head");
await tick();
assert.ok($("side").classList.contains("named-closed"), "the named cells fold away");
assert.strictEqual(doc.querySelectorAll("#named .named-row").length, 0);
press("#named-head");
await tick();
assert.ok(doc.querySelectorAll("#named .named-row").length > 0, "and unfold");

// ⌘S: boards autosave; the menu item says so.
listeners.menu({ payload: "file.save" });
await tick();
assert.match($("toast").textContent, /^Saved\. recalc saves every change as you make it\.$/, "Save reassures rather than pretending");

// The guide opens from the Help menu.
listeners.menu({ payload: "help.review" });
await tick();
assert.match($("overlay").textContent, /Signing off/, "the review guide opens");
key(doc, "Escape");
await tick();
assert.ok($("overlay").hidden);

// A full snapshot for another board resets the selection.
listeners.board({ payload: { ...snapshot, dir: "/elsewhere", cells: [mkCell({ name: "A1", value: "new" })] } });
await tick();
assert.strictEqual($("namebox").textContent, "");
assert.strictEqual(doc.querySelector("#cells .cell[data-name='A1'] .val").textContent, "new");

// A big run asks first, with the count: 22 agent cells without answers.
{
  const many = Array.from({ length: 22 }, (_, i) => mkCell({ name: `B${i + 2}`, kind: "agent_call", source: "@claude('q')", value: "", state: "stale" }));
  listeners.board({ payload: { ...snapshot, dir: "/elsewhere", full: true, cells: [mkCell({ name: "A1", value: "new" }), ...many] } });
  await tick();
  board.select("B2", "B23");
  await tick();
  calls.length = 0;
  listeners.menu({ payload: "cell.run_selection" });
  await tick();
  assert.match($("overlay").textContent, /Run 22 agent cells\?/, "a run over the threshold asks first");
  assert.match($("overlay").textContent, /no estimate/, "and says when there's no price to go on");
  assert.ok(!calls.some((c) => c.cmd === "run_selection"), "nothing runs before the answer");
  key(doc, "Escape");
  await tick();
  await tick();
  assert.ok(!calls.some((c) => c.cmd === "run_selection"), "dismissing runs nothing");
  assert.match($("inspector-body").textContent, /Nothing was run/);
  listeners.menu({ payload: "cell.run_selection" });
  await tick();
  [...$("overlay").querySelectorAll("button")].find((b) => b.textContent === "Run 22").click();
  await tick();
  await tick();
  assert.strictEqual(calls.findLast((c) => c.cmd === "run_selection")?.args.names.length, 22, "Run 22 runs them");
}

// The same permission from several runs is one row; its answer goes to each. A risky one says why.
{
  const ask = (name, what) =>
    mkCell({
      name,
      kind: "agent_call",
      source: "@claude('q')",
      state: "awaiting",
      decision: {
        question: `The agent wants to use ${what}. Allow it this once?`,
        options: [{ label: "Allow once", value: "0", preview: '{\n  "command": "x"\n}' }, { label: "Allow Bash for this run", value: "1" }, { label: "Deny", value: "2" }],
        allow_freetext: true,
        asker: `agent:${name}`,
        posed_at: Date.now(),
        from_run: true,
      },
    });
  listeners.board({ payload: { ...snapshot, dir: "/elsewhere", full: true, cells: [ask("C2", "Bash: git push origin main"), ask("C3", "Bash: git push origin main"), ask("C4", "Bash: ls")] } });
  await tick();
  listeners.menu({ payload: "agent.decisions" });
  await tick();
  const group = doc.querySelector("#tray .need-group");
  assert.ok(group, "two runs asking the same thing share a row");
  assert.match(group.textContent, /2 cells ask/);
  assert.match(group.textContent, /C2.*C3/);
  assert.match(group.textContent, /changes git history/, "and the row says what the command could do");
  assert.strictEqual(doc.querySelectorAll("#tray .need-ask:not(.need-group)").length, 1, "the different request keeps its own row");
  assert.doesNotMatch($("tray").textContent, /"command"/i, "the raw request isn't in the list");
  calls.length = 0;
  [...group.querySelectorAll("button")].find((b) => b.textContent === "Deny all 2").click();
  await tick();
  assert.deepStrictEqual(calls.filter((c) => c.cmd === "resolve_decision").map((c) => c.args), [{ name: "C2", index: 2 }, { name: "C3", index: 2 }], "the answer goes to each run");
  press("#cells .cell[data-name='C4']");
  await tick();
  assert.ok(doc.querySelector("#inspector-body details.preview-fold"), "the inspector folds the raw request");
}

// Close the window so the app's 1s timer doesn't keep node alive.
window.close();
console.log(
  "smoke ok: virtual grid, headers, in-place editor + formula bar, computed-cell edit decision, dependency highlights, autocomplete (fn/$ref/agent), F4 anchors, menu-driven run/verify/fill/delete/undo, block summary + run selection, clipboard copy/paste (internal + foreign), sessions + stale inputs + notify, decisions + needs-you queue + snooze shelf + inline actions, review mode + Review Answers, Space to expand, answer types + usage, palette/cheatsheet/gallery/recents, new named cell, context menu, delta repaint, citations + source pane, run column, question column dialog, board switch, big-run confirm, grouped permissions, session handoff + driver seat",
);

/** Open the inspector's More menu and pick an entry by its label. */
function more(label) {
  const btn = [...doc.querySelectorAll("#inspector-body button")].find((b) => b.textContent === "More");
  assert.ok(btn, "no More button in the inspector");
  btn.dispatchEvent(new window.Event("click", { bubbles: true }));
  const item = [...doc.querySelectorAll("#ctx .item")].find((i) => i.textContent.startsWith(label));
  assert.ok(item, `no "${label}" under More: ${[...doc.querySelectorAll("#ctx .item")].map((i) => i.textContent).join(", ")}`);
  item.dispatchEvent(new window.Event("click", { bubbles: true }));
}
function press(sel) {
  const el = doc.querySelector(sel);
  assert.ok(el, `no element ${sel}`);
  el.dispatchEvent(new window.MouseEvent("mousedown", { bubbles: true, button: 0 }));
  el.dispatchEvent(new window.MouseEvent("mouseup", { bubbles: true, button: 0 }));
  el.dispatchEvent(new window.MouseEvent("click", { bubbles: true, button: 0 }));
}
function nameNowTyped() {
  return $("namebox").textContent;
}
// Editing in place: the input shows, not just waiting unseen for typing.
function editorOpen() {
  const e = $("cell-editor");
  return !e.hidden && !e.classList.contains("waiting");
}
// A key as the browser delivers it: to the focused element when the grid has focus
// (the waiting cell input), and a printable key nobody cancels lands in that input.
function key(target, k, opts = {}) {
  const t = target === $("grid-wrap") && doc.activeElement?.id === "cell-editor" ? doc.activeElement : target;
  const ev = new window.KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true, ...opts });
  t.dispatchEvent(ev);
  if (!ev.defaultPrevented && (t.tagName === "INPUT" || t.tagName === "TEXTAREA") && k.length === 1 && !opts.metaKey && !opts.ctrlKey) {
    t.value += k;
    t.dispatchEvent(new window.Event("input", { bubbles: true }));
  }
}
function tick() {
  return new Promise((r) => setTimeout(r, 15));
}
