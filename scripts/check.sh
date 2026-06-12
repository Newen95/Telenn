#!/usr/bin/env sh
# Local CI: format, lint, engine tests and frontend build.
# Works in Git Bash on Windows and on Linux.
set -e

# Use cargo from PATH, falling back to the default rustup location.
CARGO="${CARGO:-cargo}"
command -v "$CARGO" >/dev/null 2>&1 || CARGO="$HOME/.cargo/bin/cargo"

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"

(
  cd "$ROOT/src-tauri"
  "$CARGO" fmt --check
  "$CARGO" clippy --all-targets -- -D warnings
  "$CARGO" test
)

cd "$ROOT"
npm run build
