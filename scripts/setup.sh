#!/usr/bin/env sh
# One-time machine setup for Telenn on Linux/macOS: system dependencies, Rust
# toolchain and npm packages. Safe to re-run. On Windows, use setup.ps1 instead.
set -e

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
os="$(uname -s)"

install_linux_deps() {
  if ! command -v apt-get >/dev/null 2>&1; then
    echo "Non-apt Linux detected. Install the Tauri system dependencies manually:"
    echo "  https://v2.tauri.app/start/prerequisites/"
    return
  fi
  echo "Installing Tauri system dependencies (sudo apt-get)..."
  sudo apt-get update
  sudo apt-get install -y \
    libwebkit2gtk-4.1-dev \
    build-essential \
    curl wget file \
    libxdo-dev \
    libssl-dev \
    libayatana-appindicator3-dev \
    librsvg2-dev
}

case "$os" in
  Linux) install_linux_deps ;;
  Darwin) echo "macOS detected. Ensure the Xcode command line tools are installed: xcode-select --install" ;;
  *) echo "Unsupported OS '$os'. On Windows, use setup.ps1." ; exit 1 ;;
esac

# Rust: install rustup if it is missing. The exact version and components come
# from rust-toolchain.toml, which rustup honours automatically.
if ! command -v cargo >/dev/null 2>&1 && [ ! -x "$HOME/.cargo/bin/cargo" ]; then
  echo "Installing rustup..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
fi

# Trigger installation of the pinned toolchain and its components.
RUSTUP="$HOME/.cargo/bin/rustup"
command -v rustup >/dev/null 2>&1 && RUSTUP="rustup"
( cd "$ROOT" && "$RUSTUP" show >/dev/null )

echo "Installing npm packages..."
( cd "$ROOT" && npm install )

echo "Setup complete. Run ./check.sh to verify."
