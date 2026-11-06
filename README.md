<p align="center">
  <img src="desktop/src-tauri/icons/128x128@2x.png" width="112" alt="recalc">
</p>

<h1 align="center">recalc</h1>

<p align="center">
  <strong>A spreadsheet as your harness of harnesses, where every cell can be used as its own terminal.</strong><br>
  Watch agents fill cells in real time, open any one in its own terminal, and steer it by prompting it or by editing the cells it fills.
</p>

<p align="center">
  macOS · Linux (experimental) · works with Claude Code, Codex and pi (experimental) · free and open source · runs on your computer
</p>

<p align="center">
  <a href="#see-it-work"><strong>See it work</strong></a> ·
  <a href="#get-started">Get started</a> ·
  <a href="#cells-in-30-seconds">Cells in 30 seconds</a> ·
  <a href="demos/README.md">Examples</a> ·
  <a href="docs/GUIDE.md">The guide</a>
</p>

> [!WARNING]
> **recalc is in early beta.** It's very, very early in development, so expect bugs, rough edges, and changes between versions that break things. Keep copies of anything you can't afford to lose, and please [tell us what breaks](../../issues).

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/images/grid-dark.png">
    <img src="docs/images/grid.png" alt="A recalc board: six contracts down the side, a question in each column, and an AI answer in every cell, with the passage it came from shown on the right">
  </picture>
</p>

## Why

Spreadsheets are still a great way to organize and analyze data. While it would be possible to just let Claude Code or Codex take control of Excel or Google Sheets, this felt like an opportunity to reimagine the spreadsheet for the agentic AI age from the ground up (though most of it should still feel familiar). This is still very experimental and very much a work in progress.

In its current form, recalc gives you and your agents one shared grid. Every cell can hold a number, a formula, a terminal command, or an AI agent. Agents write their plans and results into cells while you watch. Commands in other cells check their work and turn red when something's wrong. When you want something changed, you edit a cell instead of interrupting the chat. The different Claude Code or Codex instances running in recalc understand how recalc works via an MCP server.

## What you can do with it

**Check an agent's "done".** Put a cell that runs your tests next to the cell where the agent says it's finished. The test cell re-runs by itself whenever the code changes. If the agent says done while a test is failing, you see it straight away. You can even stop a Claude Code session from finishing until its row goes green.

**Ask a hundred documents the same questions.** Put files down the side and questions across the top, then press Run. An agent answers every cell, and each answer links to the exact passage it came from. Click a cell to see the quote highlighted in the document. Mark the answers you've checked. Agents can't overwrite them after that. Export the result to Excel when you're done, or to share it.

**Run a few agents at once without losing track.** Give each agent its own row and watch them all on one screen. Anything that needs you (a question, an error, a finished answer) lands in one list called **Needs You**. Or let the **Driver**, an agent that works the board for you, set things up from a prompt.

**Keep your work on your computer.** There's no account, no server and no tracking. The only request recalc makes on its own is a once-a-day check for a new version, which you can turn off. A board is just a folder of files. The agents you connect send what they always send to their own providers. For private documents, you can [run the agents on a local model](docs/LOCAL.md).

## See it work

**A tour of the app** (1:22). Every kind of cell, an agent column on Haiku, review and Needs You, the Driver, sheets and sharing.

https://github.com/user-attachments/assets/72b60b54-946b-4236-acd5-4850befe0b27

**Contract review** (1:40). Twelve public contracts and six questions: 72 cited answers, a wrong one caught from its own source, flagged, and fixed by re-running with the note.

https://github.com/user-attachments/assets/705e7993-eddb-477b-ae0f-c0442f809a3d

**S-corp returns** (4:16). Five fictional clients' books carried onto the IRS's draft 2026 Form 1120-S: agents classify each account, formulas build and check each return, and an action fills the PDF only once the checks pass. Run it yourself with `recalc demo scorp`.

https://github.com/user-attachments/assets/f3d12576-151d-4f5b-a0ab-e836d094aeb4

**Tying out a 10-K** (0:53). Sixteen figures pulled from a public annual report, each with the line it came from, and formulas that check they agree, down to a $14.4 million difference the notes explain.

https://github.com/user-attachments/assets/571af978-53cb-4d08-84ad-3d151bb19a89

**A model benchmark** (2:37). Models predicting FDA decisions from dossiers frozen before each decision: add a new model as a contestant, score it against the others, then open one of its answers in Terminal and ask it why.

https://github.com/user-attachments/assets/6208594e-8072-4a30-9dd7-fd4f22c50499

## Get started

1. **Install recalc.** In a terminal on a Mac (Apple silicon or Intel):

   ```
   curl -fsSL https://raw.githubusercontent.com/sshah03/recalc/main/install.sh | sh
   ```

   It puts recalc in your Applications folder and a `recalc` command on your PATH. It's a short script; [read it](install.sh) first if you like. Run it again any time to reinstall. On Linux the same line installs the AppImage, but Linux support is experimental.

   <details>
   <summary>Or install by hand</summary>

   Download the `.dmg` for your Mac from [Releases](../../releases/latest) and drag recalc to Applications. The beta isn't signed by Apple yet, so macOS refuses to open a downloaded copy until you clear its quarantine flag once:

   ```
   xattr -dr com.apple.quarantine /Applications/recalc.app
   ```

   (Or try to open it, then click Open Anyway in System Settings → Privacy & Security.) recalc → Install Command Line Tool… adds the `recalc` command.
   </details>

2. **Make sure you have an agent.** recalc runs the agents you already use. Install [Claude Code](https://claude.com/claude-code), [Codex](https://github.com/openai/codex) or [pi](https://pi.dev) (experimental) and sign in once. Agent → Connect an Agent shows what recalc found.
3. **Try an example.** File → New Board… → Examples. "Legal due diligence" is a good first one: select the empty answer cells and choose Cell → Run Selection (⇧⌘↩) to watch agents fill them in.

Your agent can also work on the board from its own chat. To connect a Claude Code you run yourself, run this in your project's folder:

```
claude mcp add recalc -- recalc mcp
```

(Without the `recalc` command, Agent → Connect an Agent shows the same line with the app's full path.) After that, just work as usual: the agent knows the board is there and uses it.

## Cells in 30 seconds

The first character of a cell says what it is:

| You type | It's | What happens |
|---|---|---|
| `42` | a value | It just sits there |
| `= {passed} > 0` | a formula | Updates when the cells it reads change |
| `$ npm test` | a command | Runs in your terminal, and its output becomes the value. Re-runs when its inputs change. (`$1,200` is just an amount) |
| `! git push` | an action | Only runs when **you** run it. Use it for anything that changes things |
| `@claude('summarize {A2}')` | an agent | Asks Claude (or `@codex`, `@pi`) and puts the answer in the cell. Only runs when you run it |

`{A2}` pulls in another cell's value. That's most of what you need to know.

## Good to know

- **Does it cost anything?** recalc is free. The agents you run use your own Claude or ChatGPT plan, the same as they do in a standalone terminal. The dollar figures recalc shows are each run's cost at API prices, as the agent reports it. On a plan nothing is billed per run; the figure tells you how much of your plan's usage a board took, and Set Agent Budget… caps it.
- **Is it safe?** Nothing that changes anything runs on its own: `!` actions and agent cells only run when you say so, and an agent can't be the first to run an action it wrote. recalc isn't a sandbox, though. Commands run with your permissions, so run it where you'd run them anyway.
- **Where are my boards?** New boards go in `~/Documents/recalc boards`, and everything saves as you go. Click a board's name at the top of the window to rename or move it.
- **Can I use it in a code project?** Yes. Open the project with File → Open Folder… and the board lives in the project, in a `.recalc` folder you can commit or ignore. To open boards from a terminal, type `recalc` in any project folder (the install script adds the command; after a hand install, choose recalc → Install Command Line Tool…).
- **How do I update?** recalc checks once a day and offers to restart into the new version. recalc → Check for Updates… checks now.
- **Why isn't it signed by Apple?** It's a beta. Releases are built in the open from this repository by GitHub Actions ([the workflow](.github/workflows/release.yml)), and updates install only when they're signed with the project's own key. Apple signing comes before a wider release.
- **Windows?** Not yet.

## Learn more

- **[The guide](docs/GUIDE.md)** covers everything in depth: formulas, document grids, reviewing, agents talking to each other, scripts, the command line, and CI.
- **[Examples](demos/README.md)**: what each example board shows, and how to reuse it.
- **[Recipes](RECIPES.md)**: boards people keep coming back to, and why they work.
- **[AGENTS.md](AGENTS.md)**: what agents are told about working on a board. Copy it into your own repo.

## Feedback

This is a beta, so your bug reports and ideas shape what comes next. [Open an issue](../../issues) with what you did, what you expected, and what happened. If recalc crashes, it writes a log to `~/Library/Logs/recalc/crash.log` on macOS (`~/.local/state/recalc/crash.log` on Linux); please attach it.

## Building from source

You'll need Rust and Node.js.

```
cd desktop/frontend && npm install && npm run build
cd ../.. && cargo build --release -p recalc-desktop     # → target/release/recalc
```

[desktop/README.md](desktop/README.md) has the development notes, and [CONTRIBUTING.md](CONTRIBUTING.md) has the checks a change needs to pass.

## License

Apache License 2.0; see [LICENSE](LICENSE) and [NOTICE](NOTICE). Copyright 2026 sshah03.
