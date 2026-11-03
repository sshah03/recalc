# Releasing

## Cutting a release

1. Set the same version in `Cargo.toml`, `desktop/src-tauri/Cargo.toml`, `desktop/src-tauri/tauri.conf.json`, and `desktop/frontend/package.json`. The release workflow refuses a tag that doesn't match all four.
2. Re-record the agent fixtures if Claude Code or Codex has released since the last ones (see below), and make sure `cargo test` passes against them.
3. Tag and push: `git tag v0.1.0 && git push origin v0.1.0`.
4. `.github/workflows/release.yml` builds macOS (Apple silicon and Intel) and Linux bundles into a **draft** release. Check the draft, write the notes from the commits since the last tag, and publish it. Write the notes for the people using it: the update dialog's What's New opens this page.

## Updates

Installed copies check `releases/latest/download/latest.json` on this repository once a day (the release workflow builds that address into the app as `RECALC_UPDATE_URL`; a build from source has none and doesn't check). They install an update only when it is signed with the update key, whose public half is `plugins.updater.pubkey` in `desktop/src-tauri/tauri.conf.json`.

- The private key was made with `cargo tauri signer generate` and lives outside the repo, at `~/.tauri/recalc-updater.key`, with no password. **Back it up.** Lose it and no installed copy can be updated again; everyone would have to download the next version by hand.
- Secrets of the `release` environment (Settings → Environments → release, which needs the maintainer's approval for each run; only the maintainer can push `v*` tags): `TAURI_SIGNING_PRIVATE_KEY` (the key file's contents) and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (empty). With them, the workflow adds `--config tauri.release.conf.json`, which makes the update bundles, and tauri-action uploads them and `latest.json` to the draft. Without them the release still builds, with a warning, and carries no update.
- A draft isn't `latest`: installed copies see a release when it's published.
- Before a release, `node desktop/e2e/update-install.mjs` builds this checkout as 0.1.0 and a signed 0.1.1, and checks that Restart to Update installs one over the other and comes back with its board. `desktop/e2e/onboard.mjs` checks the rest (the daily check, its off switch, and an update signed with another key refused).
- On Linux only the AppImage updates itself; a `.deb` gets a Download button to the release page.

## macOS signing and notarization

Without the secrets below, the macOS builds are ad-hoc signed: they run, but the first launch is blocked until the user allows it under System Settings → Privacy & Security → Open Anyway. With them, the builds are signed with a Developer ID, notarized, and stapled, and open normally.

Secrets of the `release` environment (Settings → Environments → release):

| Secret | What it holds |
|---|---|
| `APPLE_CERTIFICATE` | The Developer ID Application certificate exported as a `.p12`, base64-encoded (`base64 -i cert.p12 \| pbcopy`) |
| `APPLE_CERTIFICATE_PASSWORD` | The password the `.p12` was exported with |
| `APPLE_SIGNING_IDENTITY` | The certificate's name, e.g. `Developer ID Application: Your Name (TEAMID)` |
| `APPLE_API_ISSUER` | App Store Connect API issuer ID (Users and Access → Integrations → Keys) |
| `APPLE_API_KEY` | That API key's ID |
| `APPLE_API_KEY_P8` | The key's `.p8` file contents, pasted as-is |

The workflow checks that the stapled ticket validates on every signed `.dmg`.

## Agent fixtures

`tests/fixtures/agents/<cli>-<version>/` holds output recorded from real Claude Code and Codex runs, and `src/agent.rs` tests that recalc parses each one. A CLI release that changes its output format fails there before it fails on someone's board.

To record the installed versions:

```
scripts/record-agent-fixtures.sh
```

It runs a one-word prompt and a deliberately bad model name through each CLI (a few cents of usage) and writes the output under the CLI's version. Keep the old directories: people run older CLIs too. Add the new version to the tests in `src/agent.rs` next to the old one.

A rate limit can't be produced on demand, so `rate-limit.synthetic.json` is the recorded error with only its status changed to 429. When a real one turns up in a log, save it beside the synthetic one.
