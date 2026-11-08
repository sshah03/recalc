#!/bin/sh
# Record what the installed Claude Code and Codex print, for the parser
# tests in src/agent.rs. See docs/RELEASING.md. Costs a few cents.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cd "$work"

if command -v claude >/dev/null; then
  v=$(claude --version | awk '{print $1}')
  out="$root/tests/fixtures/agents/claude-$v"
  mkdir -p "$out"
  claude -p "Reply with the single word pong and nothing else." --output-format json > "$out/ok.json"
  claude -p "hi" --output-format json --model claude-nonexistent-model-xyz > "$out/bad-model.json" || true
  # When the recorder's login expires isn't the parser's business, nor the public's.
  sed -i.bak 's/"credential_expires_at":"[^"]*"/"credential_expires_at":"2026-01-01T00:00:00.000Z"/' "$out"/*.json && rm -f "$out"/*.json.bak
  echo "recorded $out"
fi

if command -v codex >/dev/null; then
  v=$(codex --version | awk '{print $2}')
  out="$root/tests/fixtures/agents/codex-$v"
  mkdir -p "$out"
  codex exec --skip-git-repo-check --json "Reply with the single word pong and nothing else." < /dev/null > "$out/ok.jsonl"
  codex exec --skip-git-repo-check --json -m nonexistent-model-xyz "hi" < /dev/null > "$out/bad-model.jsonl" || true
  echo "recorded $out"
fi
