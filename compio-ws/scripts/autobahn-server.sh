#!/usr/bin/env bash
set -euo pipefail
set -x

# Get to workspace root (compio/)
SOURCE_DIR=$(readlink -f "${BASH_SOURCE[0]}")
SOURCE_DIR=$(dirname "$SOURCE_DIR")
cd "${SOURCE_DIR}/../.."  # Go up to workspace root

WSSERVER_PID=""

function cleanup() {
    if [ -n "${WSSERVER_PID}" ]; then
        kill -9 ${WSSERVER_PID} 2>/dev/null || true
    fi
}
trap cleanup TERM EXIT INT

function test_diff() {
    if ! diff -q \
        <(jq -S 'del(."Tungstenite" | .. | .duration?)' 'compio-ws/autobahn/expected-results.json') \
        <(jq -S 'del(."Tungstenite" | .. | .duration?)' 'compio-ws/autobahn/server/index.json')
    then
        echo 'Difference in results, either this is a regression or' \
             'one should update compio-ws/autobahn/expected-results.json with the new results.'
        exit 64
    fi
}

cargo build --release --features connect -p compio-ws --example autobahn-server

cargo run --release --features connect -p compio-ws --example autobahn-server &
WSSERVER_PID=$!

sleep 5

docker run --rm \
    -v "${PWD}/compio-ws/autobahn:/autobahn" \
    --network host \
    crossbario/autobahn-testsuite \
    wstest -m fuzzingclient -s 'autobahn/fuzzingclient.json'

test_diff