#!/usr/bin/env bash
# Kiwi Installer (https://github.com/Tribal-Chief-001/Kiwi)
set -euo pipefail

REPO="Tribal-Chief-001/Kiwi"
INSTALL_DIR="/usr/local/bin"

echo "🥝 Installing Kiwi: Zero-Footprint CLI Network Intelligence Engine..."

# Detect OS and architecture
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"

case "$ARCH" in
  x86_64|amd64) ARCH="x86_64" ;;
  aarch64|arm64) ARCH="aarch64" ;;
  *) echo "❌ Unsupported architecture: $ARCH"; exit 1 ;;
esac

case "$OS" in
  linux) TARGET="${ARCH}-unknown-linux-musl" ;;
  darwin) TARGET="${ARCH}-apple-darwin" ;;
  *) echo "❌ Unsupported operating system: $OS"; exit 1 ;;
esac

# Check for Cargo if binary releases are not yet published
if command -v cargo >/dev/null 2>&1; then
  echo "📦 Cargo detected. Building and installing Kiwi from source..."
  cargo install --git "https://github.com/${REPO}.git" --bin kiwi
  echo "✅ Kiwi successfully installed! Run 'kiwi' to test your network."
  exit 0
fi

echo "❌ Please install Rust via https://rustup.rs/ or download pre-built releases from https://github.com/${REPO}/releases"
exit 1
