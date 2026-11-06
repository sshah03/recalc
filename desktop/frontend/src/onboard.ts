// A first run that says what's missing, and keeping recalc current: which
// agents are ready (on the start screen), Install Command Line Tool…, and
// updates (Check for Updates…, the status bar's "Update to …").

import { closeOverlay, openOverlay, toast } from "./chrome.js";
import { board } from "./state.js";
import * as api from "./tauri.js";

/** Words with `code` spans: ["Type ", ["recalc"], " in a folder."]. */
function para(...parts: (string | [string])[]): HTMLElement {
  const p = document.createElement("p");
  for (const part of parts) {
    if (typeof part === "string") p.append(part);
    else {
      const c = document.createElement("code");
      c.textContent = part[0];
      p.appendChild(c);
    }
  }
  return p;
}

function button(label: string, onclick: () => void, primary = false): HTMLButtonElement {
  const b = document.createElement("button");
  b.className = primary ? "act primary" : "act";
  b.textContent = label;
  b.onclick = onclick;
  return b;
}

function dialog(title: string): { body: HTMLElement; acts: HTMLElement } {
  const body = document.createElement("div");
  body.className = "welcome-intro onboard";
  body.textContent = "Checking…";
  const acts = document.createElement("div");
  acts.className = "acts modal-acts";
  openOverlay(title, body, acts);
  return { body, acts };
}

const message = (e: unknown) => String((e as Error)?.message ?? e);

// ---- agents on the start screen ----

/** What the start screen says about agents: nothing when one is ready and
 * none is signed out; otherwise which aren't ready and what to do. */
export function agentNote(): HTMLElement {
  const box = document.createElement("div");
  box.className = "agent-note";
  box.hidden = true;
  const fill = () =>
    void api
      .agentStatus()
      .then((agents) => {
        const ready = agents.filter((a) => a.found && a.signed_in !== false);
        const signedOut = agents.filter((a) => a.found && a.signed_in === false);
        if (ready.length && !signedOut.length) {
          box.hidden = true;
          return;
        }
        const lead = document.createElement("p");
        lead.className = "agent-note-lead";
        lead.textContent = ready.length
          ? "An agent is installed but not signed in."
          : "recalc runs the AI agents you already use. Install Claude Code, Codex or pi and sign in once to use agent cells and the Driver. Formulas and commands work without one.";
        const rows = (ready.length ? signedOut : agents).map((a) => {
          const r = document.createElement("div");
          r.className = "agent-note-row";
          const name = document.createElement("span");
          name.className = "agent-name";
          name.textContent = a.product;
          r.appendChild(name);
          if (!a.found) {
            const state = document.createElement("span");
            state.textContent = "Not installed";
            r.append(state, button(`Get ${a.product}`, () => void api.openUrl(a.install).catch((e) => toast(message(e)))));
          } else {
            r.appendChild(para("Not signed in. Open Terminal and run ", [a.name], " once."));
          }
          return r;
        });
        const again = button("Check Again", fill);
        again.classList.add("small");
        box.replaceChildren(lead, ...rows, again);
        box.hidden = false;
      })
      .catch(() => (box.hidden = true));
  fill();
  return box;
}

// ---- Install Command Line Tool ----

export async function openCli() {
  const { body, acts } = dialog("Command line tool");
  const done = button("Done", closeOverlay);
  let st: api.CliStatus;
  try {
    st = await api.cliStatus();
  } catch (e) {
    body.replaceChildren(para(message(e)));
    acts.replaceChildren(done);
    return;
  }
  const what = para("Lets you type ", ["recalc"], " in a terminal: ", ["recalc"], " in a project folder opens its board, and ", ["recalc --help"], " lists the rest.");
  const install = (replace: boolean) => async () => {
    acts.querySelectorAll("button").forEach((b) => (b.disabled = true));
    try {
      const at = await api.installCli(replace);
      body.replaceChildren(para("Installed at ", [at], ". Open a new terminal window and type ", ["recalc"], "."));
      if (!st.on_path) body.appendChild(para(`${at.replace(/\/recalc$/, "")} isn't on your terminal's PATH yet. Add it in your shell's startup file.`));
      acts.replaceChildren(done);
    } catch (e) {
      body.appendChild(para(message(e)));
      acts.querySelectorAll("button").forEach((b) => (b.disabled = false));
    }
  };
  if (st.blocked) {
    body.replaceChildren(para(st.blocked));
    acts.replaceChildren(done);
  } else if (st.installed) {
    body.replaceChildren(para("Installed: a terminal finds ", ["recalc"], ` at ${st.existing?.path ?? st.target}.`), what);
    acts.replaceChildren(done);
  } else if (st.existing && st.existing.link_to === null) {
    body.replaceChildren(para(`There's already a recalc program at ${st.existing.path}. Remove or rename it, then try again.`));
    acts.replaceChildren(done);
  } else if (st.existing) {
    body.replaceChildren(
      para(`There's already a recalc at ${st.existing.path}, linked to ${st.existing.link_to}.`),
      para("Replace it with a link to this app?"),
    );
    acts.replaceChildren(button("Replace", install(true), true), button("Cancel", closeOverlay));
  } else {
    body.replaceChildren(what, para(`Puts a link to this app at ${st.target}.`));
    if (st.needs_admin) body.appendChild(para("macOS asks for your password: only an administrator can add to that folder."));
    acts.replaceChildren(button("Install", install(false), true), button("Cancel", closeOverlay));
  }
}

// ---- updates ----

let found: api.UpdateInfo | null = null;

/** Listen for the daily check's news and the download's progress. */
export function initUpdates() {
  void api.listen<api.UpdateInfo>("update-available", (e) => {
    const fresh = found?.version !== e.payload.version;
    found = e.payload;
    showChip();
    if (fresh) toast(`recalc ${found.version} is available. Click Update in the status bar.`);
  });
}

function showChip() {
  let chip = document.getElementById("status-update");
  if (!found) {
    chip?.remove();
    return;
  }
  if (!chip) {
    chip = document.createElement("button");
    chip.id = "status-update";
    chip.className = "status";
    chip.onclick = () => void openUpdates(false);
    document.getElementById("status-awaiting")?.before(chip);
  }
  chip.textContent = `Update to ${found.version}`;
  chip.title = `recalc ${found.version} is available. You have ${found.current}.`;
}

/** Check for Updates…; `check` false shows what the daily check found. */
export async function openUpdates(check = true) {
  const { body, acts } = dialog("Updates");
  const done = button("Done", closeOverlay);
  let st: { current: string; auto: boolean; feed: boolean };
  try {
    st = await api.updateStatus();
  } catch (e) {
    body.replaceChildren(para(message(e)));
    acts.replaceChildren(done);
    return;
  }
  const auto = document.createElement("label");
  auto.className = "onboard-check";
  const box = document.createElement("input");
  box.type = "checkbox";
  box.checked = st.auto;
  box.onchange = () => void api.setAutoUpdate(box.checked).catch((e) => toast(message(e)));
  auto.append(box, " Check for updates once a day");
  if (!st.feed) {
    body.replaceChildren(para(`This copy of recalc (${st.current}) was built from source, so it doesn't check for updates.`));
    acts.replaceChildren(done);
    return;
  }
  if (check || !found) {
    try {
      found = await api.checkUpdate();
    } catch (e) {
      body.replaceChildren(para(message(e)), auto);
      acts.replaceChildren(done);
      return;
    }
    showChip();
  }
  if (!found) {
    body.replaceChildren(para(`You have the newest version, recalc ${st.current}.`), auto);
    acts.replaceChildren(done);
    return;
  }
  const u = found;
  body.replaceChildren(para(`recalc ${u.version} is available. You have ${u.current}.`));
  const whatsNew = u.page ? [button("What's New", () => void api.openUrl(u.page!).catch((e) => toast(message(e))))] : [];
  if (u.notes.trim()) {
    const notes = document.createElement("div");
    notes.className = "update-notes";
    notes.textContent = u.notes.trim();
    body.appendChild(notes);
  }
  if (!u.can_install) {
    body.append(para(u.why_not ?? "This copy can't update itself."), auto);
    acts.replaceChildren(...(u.page ? [button("Download", () => void api.openUrl(u.page!).catch((e) => toast(message(e))), true)] : []), button("Later", closeOverlay));
    return;
  }
  const running = [...board.cells.values()].filter((c) => c.state === "computing" || c.state === "queued").length;
  if (running) body.appendChild(para(`${running} ${running === 1 ? "cell is" : "cells are"} running. Restarting stops ${running === 1 ? "it" : "them"}; run ${running === 1 ? "it" : "them"} again afterwards.`));
  body.appendChild(auto);
  const status = document.createElement("p");
  status.className = "update-progress";
  const go = button("Restart to Update", async () => {
    acts.querySelectorAll("button").forEach((b) => (b.disabled = true));
    status.textContent = "Downloading…";
    body.appendChild(status);
    const off = await api.listen<{ done: number; total: number | null }>("update-progress", (e) => {
      const { done: d, total } = e.payload;
      status.textContent = total ? `Downloading… ${Math.floor((d / total) * 100)}%` : `Downloading… ${(d / 1e6).toFixed(1)} MB`;
    });
    try {
      await api.installUpdate(); // restarts on success
    } catch (e) {
      off();
      status.textContent = message(e);
      acts.querySelectorAll("button").forEach((b) => (b.disabled = false));
    }
  }, true);
  acts.replaceChildren(go, ...whatsNew, button("Later", closeOverlay));
}
