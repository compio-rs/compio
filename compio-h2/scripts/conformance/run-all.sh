#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
PASS=0; FAIL=0; SKIP=0

run_suite() {
    local name="$1" script="$2"
    echo ""
    echo "========================================"
    echo "  $name"
    echo "========================================"
    if [[ -x "$script" ]]; then
        if "$script"; then
            echo "  -> PASS"
            PASS=$((PASS+1))
        else
            echo "  -> FAIL"
            FAIL=$((FAIL+1))
        fi
    else
        echo "  -> SKIP (not found or not executable)"
        SKIP=$((SKIP+1))
    fi
}

# Check prerequisites and skip gracefully
check_prereq() {
    local name="$1" cmd="$2"
    if ! command -v "$cmd" &>/dev/null; then
        echo "NOTE: $cmd not found, $name tests will use run.sh which checks prereqs"
    fi
}

check_prereq "h2spec" "h2spec"
check_prereq "h2load" "h2load"

run_suite "h2spec conformance"      "$SCRIPT_DIR/run-h2spec.sh"
run_suite "h2load load testing"     "$SCRIPT_DIR/run-h2load.sh"

echo ""
echo "========================================"
echo "  RESULTS: $PASS passed, $FAIL failed, $SKIP skipped"
echo "========================================"
[[ $FAIL -eq 0 ]]
