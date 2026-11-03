#!/bin/sh
# Installs recalc from its GitHub releases:
#   curl -fsSL https://raw.githubusercontent.com/sshah03/recalc/main/install.sh | sh
#
# macOS: puts recalc.app in /Applications (or ~/Applications) and a `recalc`
# command on your PATH. Linux (experimental): the AppImage in
# ~/.local/share/recalc and `recalc` in ~/.local/bin.
#
# recalc is in beta and its builds aren't signed by Apple yet. A browser
# download would be quarantined and refused by macOS; one fetched with curl
# isn't, and this script clears the flag anyway.
#
# Settings, all optional:
#   RECALC_VERSION=v0.1.0   a release tag (default: the latest)
#   RECALC_APPS=~/Apps      where recalc.app goes (macOS)
#   RECALC_BIN=~/bin        where the `recalc` command goes
#   RECALC_REPO=owner/repo  the GitHub repository to install from
set -eu

REPO="${RECALC_REPO:-sshah03/recalc}"
VERSION="${RECALC_VERSION:-latest}"
API="${RECALC_API:-https://api.github.com}"

say() { printf '%s\n' "$*"; }
fail() {
  printf 'recalc install: %s\n' "$*" >&2
  exit 1
}
need() { command -v "$1" >/dev/null 2>&1 || fail "needs $1, which isn't installed"; }

need curl
tmp=$(mktemp -d)
mounted=""
cleanup() {
  [ -n "$mounted" ] && hdiutil detach -quiet "$mounted" >/dev/null 2>&1 || true
  rm -rf "$tmp"
}
trap cleanup EXIT INT TERM

# ---- which file ----
os=$(uname -s)
arch=$(uname -m)
case "$os/$arch" in
  Darwin/arm64) pattern='_aarch64\.dmg$' ;;
  Darwin/x86_64) pattern='_x64\.dmg$' ;;
  Linux/x86_64) pattern='_amd64\.AppImage$' ;;
  Linux/aarch64) pattern='_aarch64\.AppImage$' ;;
  *) fail "no build for $os on $arch yet" ;;
esac

if [ "$VERSION" = latest ]; then
  release="$API/repos/$REPO/releases/latest"
else
  release="$API/repos/$REPO/releases/tags/$VERSION"
fi
say "Finding recalc ($VERSION) for $os ${arch}…"
curl -fsSL "$release" -o "$tmp/release.json" || fail "couldn't read $release"
# One field a line, however the JSON is laid out.
tr ',{}' '\n\n\n' <"$tmp/release.json" >"$tmp/fields"
tag=$(sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' "$tmp/fields" | head -n 1)
url=$(sed -n 's/.*"browser_download_url": *"\([^"]*\)".*/\1/p' "$tmp/fields" | grep -E "$pattern" | head -n 1 || true)
[ -n "$url" ] || fail "release ${tag:-$VERSION} has no file for $os $arch"
file="$tmp/$(basename "$url")"
say "Downloading $(basename "$url")…"
curl -fSL --progress-bar "$url" -o "$file" || fail "couldn't download $url"

# ---- where the command goes ----
# A folder already on PATH that you can write, ~/.local/bin first.
pick_bin() {
  if [ -n "${RECALC_BIN:-}" ]; then
    printf '%s\n' "$RECALC_BIN"
    return
  fi
  # Your own folders count even before they exist; a system one must be writable.
  for d in "$HOME/.local/bin" "$HOME/bin"; do
    case ":$PATH:" in *":$d:"*) printf '%s\n' "$d" && return ;; esac
  done
  for d in /usr/local/bin /opt/homebrew/bin; do
    case ":$PATH:" in *":$d:"*) [ -d "$d" ] && [ -w "$d" ] && printf '%s\n' "$d" && return ;; esac
  done
  printf '%s\n' "$HOME/.local/bin"
}

# Link `recalc` to the app. Never replaces a real program; a link to an
# older recalc is replaced.
link_cli() {
  target=$1
  bin=$(pick_bin)
  mkdir -p "$bin"
  link="$bin/recalc"
  if [ -e "$link" ] && [ ! -L "$link" ]; then
    say "Left $link alone: it's a program, not a link. Remove it to have \`recalc\` run this install."
    return
  fi
  ln -sfn "$target" "$link"
  say "Linked $link"
  case ":$PATH:" in
    *":$bin:"*) ;;
    *) say "Add $bin to your PATH to type \`recalc\` in a terminal (in ~/.zshrc: export PATH=\"$bin:\$PATH\")." ;;
  esac
}

# ---- install ----
if [ "$os" = Darwin ]; then
  need hdiutil
  apps="${RECALC_APPS:-/Applications}"
  if [ -z "${RECALC_APPS:-}" ] && [ ! -w "$apps" ]; then
    apps="$HOME/Applications"
  fi
  mkdir -p "$apps"
  # An open copy would keep running the old build.
  if pgrep -f "$apps/recalc.app/Contents/MacOS/recalc" >/dev/null 2>&1; then
    say "Quitting the recalc that's open, to replace it…"
    osascript -e 'tell application "recalc" to quit' >/dev/null 2>&1 || true
    sleep 2
  fi
  mkdir -p "$tmp/mnt"
  hdiutil attach -nobrowse -readonly -quiet -mountpoint "$tmp/mnt" "$file" || fail "couldn't open the disk image"
  mounted="$tmp/mnt"
  [ -d "$tmp/mnt/recalc.app" ] || fail "the disk image has no recalc.app"
  rm -rf "$apps/recalc.app"
  ditto "$tmp/mnt/recalc.app" "$apps/recalc.app"
  hdiutil detach -quiet "$tmp/mnt" >/dev/null 2>&1 || true
  mounted=""
  xattr -dr com.apple.quarantine "$apps/recalc.app" 2>/dev/null || true
  say "Installed recalc ${tag:-} in $apps"
  link_cli "$apps/recalc.app/Contents/MacOS/recalc"
  open_hint="open \"$apps/recalc.app\""
else
  share="$HOME/.local/share/recalc"
  mkdir -p "$share"
  mv "$file" "$share/recalc.AppImage"
  chmod +x "$share/recalc.AppImage"
  say "Installed recalc ${tag:-} in $share (Linux support is experimental)"
  link_cli "$share/recalc.AppImage"
  open_hint="recalc"
fi

say ""
say "Next:"
say "$(printf '  %-36s # %s' "$open_hint" "opens recalc")"
say "$(printf '  %-36s # %s' "claude mcp add recalc -- recalc mcp" "in a project, connects Claude Code to its board")"
say "recalc checks for updates once a day and offers to restart into them."
