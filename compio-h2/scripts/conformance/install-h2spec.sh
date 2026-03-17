#!/usr/bin/env bash
set -euo pipefail

# Install h2spec for HTTP/2 conformance testing.
# https://github.com/summerwind/h2spec
#
# Usage: ./scripts/conformance/install-h2spec.sh [INSTALL_DIR]
#   INSTALL_DIR defaults to ~/.local/bin

VERSION="2.6.0"
INSTALL_DIR="${1:-$HOME/.local/bin}"

OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"
case "$ARCH" in
    x86_64)  ARCH="amd64" ;;
    aarch64) ARCH="arm64" ;;
    arm64)   ARCH="arm64" ;;
    *)
        echo "ERROR: unsupported architecture: $ARCH"
        exit 1
        ;;
esac

URL="https://github.com/summerwind/h2spec/releases/download/v${VERSION}/h2spec_${OS}_${ARCH}.tar.gz"

echo "Installing h2spec v${VERSION} (${OS}/${ARCH}) to ${INSTALL_DIR}..."
mkdir -p "$INSTALL_DIR"
curl -sL "$URL" | tar xz -C "$INSTALL_DIR" h2spec
chmod +x "$INSTALL_DIR/h2spec"

echo "Installed: $("$INSTALL_DIR/h2spec" --version)"
echo ""
echo "Make sure $INSTALL_DIR is in your PATH:"
echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
