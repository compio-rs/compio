#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib.sh"

PORT="${H2_PORT:-8080}"
H2SPEC="${H2SPEC:-h2spec}"
TIMEOUT="${H2_STARTUP_TIMEOUT:-10}"

if ! command -v "$H2SPEC" &>/dev/null; then
    echo "ERROR: $H2SPEC not found. Run: bash crates/compio-h2/scripts/conformance/install-h2spec.sh"
    exit 1
fi

build_test_server
start_test_server "$PORT"
wait_for_server "$PORT" "$TIMEOUT"

echo "==> Running h2spec (port $PORT, strict mode, 5s timeout)..."
"$H2SPEC" -p "$PORT" --strict -o 5
STATUS=$?

if [[ $STATUS -eq 0 ]]; then
    echo "==> PASS: all h2spec tests passed"
else
    echo "==> FAIL: h2spec exited with status $STATUS"
fi

exit $STATUS
