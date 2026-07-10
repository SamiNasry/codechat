#!/usr/bin/env bash
#
# CodeChat installer — one command from zero to worldwide chat:
#
#   curl -fsSL https://raw.githubusercontent.com/SamiNasry/codechat/main/install.sh | bash
#
# What it does:
#   1. Downloads the prebuilt codechat-tui binary for your OS/arch from the
#      latest GitHub release (or builds from source if you have cargo and
#      there's no prebuilt for your platform).
#   2. Downloads the codechat wrapper script.
#   3. Installs both into ~/.local/bin.
#   4. Tells you exactly what's still missing (tmux? claude? PATH?).
#
# Override knobs:
#   CODECHAT_REPO=user/repo       install from a fork
#   CODECHAT_BIN_DIR=/some/path   install somewhere else

set -euo pipefail

REPO="${CODECHAT_REPO:-SamiNasry/codechat}"
BIN_DIR="${CODECHAT_BIN_DIR:-$HOME/.local/bin}"
BASE="https://github.com/$REPO/releases/latest/download"

say()  { printf '\033[1;35mcodechat\033[0m %s\n' "$*"; }
fail() { printf '\033[1;31mcodechat\033[0m %s\n' "$*" >&2; exit 1; }

command -v curl >/dev/null 2>&1 || fail "curl is required to install."

# ---- pick the release asset for this platform -------------------------------
OS=$(uname -s)
ARCH=$(uname -m)
case "$OS-$ARCH" in
  Linux-x86_64)              TARGET="x86_64-unknown-linux-musl" ;;
  Linux-aarch64|Linux-arm64) TARGET="aarch64-unknown-linux-musl" ;;
  Darwin-arm64)              TARGET="aarch64-apple-darwin" ;;
  Darwin-x86_64)             TARGET="x86_64-apple-darwin" ;;
  *)                         TARGET="" ;;
esac

mkdir -p "$BIN_DIR"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

# ---- get codechat-tui: prebuilt if possible, source build as fallback -------
got_binary=0
if [ -n "$TARGET" ]; then
  say "downloading codechat-tui ($TARGET)…"
  if curl -fsSL -o "$TMP/codechat-tui" "$BASE/codechat-tui-$TARGET"; then
    got_binary=1
  else
    say "no prebuilt binary for $TARGET in the latest release."
  fi
fi

if [ "$got_binary" = 0 ]; then
  if command -v cargo >/dev/null 2>&1 && command -v git >/dev/null 2>&1; then
    say "building from source instead (this takes a couple of minutes)…"
    git clone --depth 1 "https://github.com/$REPO" "$TMP/src"
    (cd "$TMP/src/tui" && cargo build --release)
    cp "$TMP/src/tui/target/release/codechat-tui" "$TMP/codechat-tui"
  else
    fail "no prebuilt binary for $OS/$ARCH and no cargo+git to build from source.
         Install Rust (https://rustup.rs) and rerun, or build manually — see the README."
  fi
fi

# ---- get the wrapper script --------------------------------------------------
say "downloading the codechat wrapper…"
curl -fsSL -o "$TMP/codechat" "$BASE/codechat" \
  || fail "could not download the codechat script from the release."

# ---- install -----------------------------------------------------------------
install -m 755 "$TMP/codechat-tui" "$BIN_DIR/codechat-tui"
install -m 755 "$TMP/codechat"     "$BIN_DIR/codechat"
say "installed codechat + codechat-tui into $BIN_DIR"

# ---- final checks: tell the user exactly what's left -------------------------
missing=0

if ! command -v tmux >/dev/null 2>&1; then
  missing=1
  say "⚠ tmux is not installed (required):"
  say "    Linux:  sudo apt install tmux"
  say "    macOS:  brew install tmux"
fi

if ! command -v claude >/dev/null 2>&1; then
  missing=1
  say "⚠ Claude Code not found on PATH: https://claude.com/claude-code"
fi

case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *)
    missing=1
    say "⚠ $BIN_DIR is not on your PATH — add this to ~/.bashrc or ~/.zshrc:"
    say "    export PATH=\"$BIN_DIR:\$PATH\""
    ;;
esac

echo
if [ "$missing" = 0 ]; then
  say "✔ all set! Run:  codechat"
else
  say "fix the ⚠ items above, then run:  codechat"
fi
say "tip: add  alias claude='codechat --no-chat'  to your shell rc,"
say "     then plain 'claude' stays vanilla and 'claude --chat' opens the chat."
