#!/usr/bin/env bash
# Shared library for external test scripts.
# Source this file from any script under scripts/:
#   source "$SCRIPT_DIR/../conformance/lib.sh"

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SERVER_PID=""

# Build the h2-server example binary.
build_test_server() {
    echo "==> Building h2-server..."
    cargo build -p compio-h2 --example h2-server

    TARGET_DIR="$(cargo metadata --format-version 1 --no-deps 2>/dev/null \
        | grep -o '"target_directory":"[^"]*"' | cut -d'"' -f4 \
        || echo "target")"
    TARGET_DIR="${TARGET_DIR:-target}"

    # Cargo example names use hyphens but the binary on disk may use underscores.
    TEST_SERVER_BIN="$TARGET_DIR/debug/examples/h2-server"
    if [[ ! -x "$TEST_SERVER_BIN" ]]; then
        TEST_SERVER_BIN="$TARGET_DIR/debug/examples/h2_server"
    fi
    if [[ ! -x "$TEST_SERVER_BIN" ]]; then
        echo "ERROR: h2-server binary not found in $TARGET_DIR/debug/examples/"
        exit 1
    fi
    export TEST_SERVER_BIN
}

# Start the test server on the given port (default 8080).
# Sets SERVER_PID and registers a cleanup trap.
start_test_server() {
    local port="${1:-8080}"
    trap stop_test_server EXIT
    echo "==> Starting h2-server on port $port..."
    "$TEST_SERVER_BIN" "$port" &
    SERVER_PID=$!
}

# Wait for the server to accept TCP connections.
# Usage: wait_for_server PORT [TIMEOUT_SECS]
wait_for_server() {
    local port="${1:?port required}"
    local timeout="${2:-10}"
    local deadline=$((SECONDS + timeout))

    while (( SECONDS < deadline )); do
        if command -v nc &>/dev/null; then
            if nc -z 127.0.0.1 "$port" 2>/dev/null; then
                echo "==> Server ready (PID $SERVER_PID)"
                return 0
            fi
        elif bash -c "echo >/dev/tcp/127.0.0.1/$port" 2>/dev/null; then
            echo "==> Server ready (PID $SERVER_PID)"
            return 0
        fi
        if ! kill -0 "$SERVER_PID" 2>/dev/null; then
            echo "ERROR: server exited before becoming ready"
            exit 1
        fi
        sleep 0.2
    done

    echo "ERROR: server did not become ready within ${timeout}s"
    exit 1
}

# Stop the test server if running.
stop_test_server() {
    if [[ -n "${SERVER_PID:-}" ]]; then
        kill "$SERVER_PID" 2>/dev/null || true
        # Wait up to 2s for graceful shutdown, then SIGKILL.
        local i=0
        while (( i < 10 )) && kill -0 "$SERVER_PID" 2>/dev/null; do
            sleep 0.2
            i=$((i + 1))
        done
        if kill -0 "$SERVER_PID" 2>/dev/null; then
            kill -9 "$SERVER_PID" 2>/dev/null || true
        fi
        wait "$SERVER_PID" 2>/dev/null || true
        SERVER_PID=""
    fi
}
