#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib.sh"

PORT="${H2_PORT:-8080}"
H2LOAD="${H2LOAD:-h2load}"
REQUESTS="${H2LOAD_REQUESTS:-10000}"
CLIENTS="${H2LOAD_CLIENTS:-10}"
STREAMS="${H2LOAD_STREAMS:-100}"
TIMEOUT="${H2_STARTUP_TIMEOUT:-10}"

if ! command -v "$H2LOAD" &>/dev/null; then
    echo "ERROR: $H2LOAD not found. Run: bash compio-h2/scripts/conformance/install-nghttp2.sh"
    exit 1
fi

build_test_server
start_test_server "$PORT"
wait_for_server "$PORT" "$TIMEOUT"

echo "==> Running h2load (port $PORT, $REQUESTS requests, $CLIENTS clients, $STREAMS streams)..."
"$H2LOAD" "http://localhost:$PORT/" -n "$REQUESTS" -c "$CLIENTS" -m "$STREAMS"
STATUS=$?

if [[ $STATUS -eq 0 ]]; then
    echo "==> PASS: h2load benchmark completed successfully"
else
    echo "==> FAIL: h2load exited with status $STATUS"
fi

exit $STATUS
