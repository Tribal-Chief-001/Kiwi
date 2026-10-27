#!/usr/bin/env bash
# Kiwi One-Line Universal Installer (Linux & macOS)
# https://github.com/Tribal-Chief-001/Kiwi
set -euo pipefail

REPO="Tribal-Chief-001/Kiwi"
VERSION="latest"

echo "🥝 Installing Kiwi: Zero-Footprint CLI Network Intelligence Engine..."

# 1. Detect OS
OS_RAW="$(uname -s | tr '[:upper:]' '[:lower:]')"
case "$OS_RAW" in
  linux*) OS="linux" ;;
  darwin*) OS="macos" ;;
  *) echo "❌ Unsupported operating system: $OS_RAW"; exit 1 ;;
esac

# 2. Detect Architecture
ARCH_RAW="$(uname -m)"
case "$ARCH_RAW" in
  x86_64|amd64) ARCH="x86_64" ;;
  arm64|aarch64) ARCH="arm64" ;;
  *) echo "❌ Unsupported architecture: $ARCH_RAW"; exit 1 ;;
esac

# Formulate artifact name
# Matches release assets: kiwi-linux-x86_64.tar.gz, kiwi-macos-arm64.tar.gz, kiwi-macos-x86_64.tar.gz
TARBALL="kiwi-${OS}-${ARCH}.tar.gz"
DOWNLOAD_URL="https://github.com/${REPO}/releases/latest/download/${TARBALL}"

TEMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TEMP_DIR"' EXIT

echo "⬇️  Downloading ${TARBALL} from GitHub Releases..."
if curl -fsSL "$DOWNLOAD_URL" -o "$TEMP_DIR/$TARBALL"; then
  echo "📦 Extracting binary..."
  tar -xzf "$TEMP_DIR/$TARBALL" -C "$TEMP_DIR"
  BINARY="$TEMP_DIR/kiwi"
elif command -v cargo >/dev/null 2>&1; then
  echo "⚠️  Pre-built release asset not found. Cargo detected! Compiling from source..."
  cargo install --git "https://github.com/${REPO}.git" --bin kiwi
  echo "🎉 Kiwi successfully installed! Type 'kiwi' from anywhere to run."
  exit 0
else
  echo "❌ Failed to download pre-built release asset from $DOWNLOAD_URL."
  echo "Please check https://github.com/${REPO}/releases"
  exit 1
fi

chmod +x "$BINARY"

# 3. Determine Installation Directory
INSTALL_DIR="/usr/local/bin"
USE_SUDO=0

if [ -w "$INSTALL_DIR" ]; then
  cp "$BINARY" "$INSTALL_DIR/kiwi"
elif command -v sudo >/dev/null 2>&1 && [ -t 0 ]; then
  echo "🔐 Root permissions required to install to $INSTALL_DIR:"
  sudo cp "$BINARY" "$INSTALL_DIR/kiwi"
else
  INSTALL_DIR="$HOME/.local/bin"
  mkdir -p "$INSTALL_DIR"
  cp "$BINARY" "$INSTALL_DIR/kiwi"

  # Ensure ~/.local/bin is in PATH
  if [[ ":$PATH:" != *":$HOME/.local/bin:"* ]]; then
    SHELL_PROFILE="$HOME/.bashrc"
    if [ -f "$HOME/.zshrc" ]; then
      SHELL_PROFILE="$HOME/.zshrc"
    fi
    echo 'export PATH="$HOME/.local/bin:$PATH"' >> "$SHELL_PROFILE"
    export PATH="$HOME/.local/bin:$PATH"
    echo "💡 Added $HOME/.local/bin to $SHELL_PROFILE"
  fi
fi

echo ""
echo "════════════════════════════════════════════════════════════════"
echo "  🎉 Kiwi successfully installed to $INSTALL_DIR/kiwi!"
echo "  ⚡ Run 'kiwi' from any terminal anytime to test your network."
echo "════════════════════════════════════════════════════════════════"
echo ""
