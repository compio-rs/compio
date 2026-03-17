#!/usr/bin/env bash
set -euo pipefail

# Install nghttp2 client tools (h2load, nghttp) for HTTP/2 testing.
# https://nghttp2.org/
#
# Usage: ./scripts/conformance/install-nghttp2.sh

OS="$(uname -s)"

echo "Installing nghttp2 client tools..."

case "$OS" in
    Linux)
        if command -v apt-get &>/dev/null; then
            sudo apt-get update
            sudo apt-get install -y nghttp2-client
        else
            echo "ERROR: unsupported Linux distribution (no apt-get found)"
            echo "Install nghttp2 manually: https://nghttp2.org/"
            exit 1
        fi
        ;;
    Darwin)
        if command -v brew &>/dev/null; then
            brew install nghttp2
        else
            echo "ERROR: Homebrew not found. Install it from https://brew.sh/"
            exit 1
        fi
        ;;
    *)
        echo "ERROR: unsupported OS: $OS"
        echo "Install nghttp2 manually: https://nghttp2.org/"
        exit 1
        ;;
esac

echo ""
echo "Verifying installation..."
h2load --version
nghttp --version
echo ""
echo "nghttp2 client tools installed successfully."
