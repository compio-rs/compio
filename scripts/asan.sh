#!/usr/bin/env bash
# Run workspace tests with AddressSanitizer (ASan) + LeakSanitizer (LSan) to
# detect memory leaks, use-after-free, buffer overflows, and stack overflows.
#
# Requires: nightly toolchain with rust-src component
#   rustup component add rust-src --toolchain nightly
#
# Usage:
#   ./scripts/asan.sh <target>             # run all tests
#   ./scripts/asan.sh <target> -- basic    # filter by test name
#   ./scripts/asan.sh --suppress <target>  # run and add all detected leaks to asan.supp
#
# Set ASAN_VERBOSE=1 to see full cargo test output.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SUPP_FILE="$SCRIPT_DIR/asan.supp"
SUPPRESS_MODE=0

if [[ "${1:-}" == "--suppress" ]]; then
    SUPPRESS_MODE=1
    shift
fi

TARGET="${1:?Usage: $0 [--suppress] <target-triple> [-- test-filter]}"
shift

# ASan + LSan: detect memory errors and leaks at process exit.
export RUSTFLAGS="${RUSTFLAGS:-} -Zsanitizer=address"
export ASAN_OPTIONS="${ASAN_OPTIONS:-detect_leaks=1:halt_on_error=0}"
export LSAN_OPTIONS="${LSAN_OPTIONS:-suppressions=$SUPP_FILE}"

# Symbolizer is required for LSan suppressions (they match on function names).
if [[ -z "${ASAN_SYMBOLIZER_PATH:-}" ]]; then
    for candidate in llvm-symbolizer llvm-symbolizer-{20,19,18,17,16,15}; do
        if command -v "$candidate" &>/dev/null; then
            export ASAN_SYMBOLIZER_PATH="$(command -v "$candidate")"
            break
        fi
    done
fi

# In suppress mode, disable existing suppressions so we see all leaks.
if [[ "$SUPPRESS_MODE" == "1" ]]; then
    export LSAN_OPTIONS=""
fi

EXTRA_ARGS=(--workspace "$@")

echo "=== ASan + LSan: detecting memory errors and leaks (workspace) ==="
echo "Target: $TARGET"
echo ""

TMPFILE=$(mktemp)
trap 'rm -f "$TMPFILE"' EXIT

# -Zbuild-std instruments std with ASan (prevents false positives and
# compilation errors in deps like zerocopy that depend on std internals).
if [[ "${ASAN_VERBOSE:-0}" == "1" ]]; then
    cargo +nightly test -Zbuild-std --target "$TARGET" "${EXTRA_ARGS[@]}" 2>&1 | tee "$TMPFILE"
else
    CARGO_EXIT=0
    cargo +nightly test -Zbuild-std --target "$TARGET" "${EXTRA_ARGS[@]}" > "$TMPFILE" 2>&1 || CARGO_EXIT=$?

    # If cargo failed and produced no test results, it's a compilation error — show full output.
    if [[ "$CARGO_EXIT" -ne 0 ]] && ! grep -q '^test result:' "$TMPFILE"; then
        cat "$TMPFILE"
        echo ""
        echo "FAILED: cargo exited with code $CARGO_EXIT (compilation error?)"
        exit "$CARGO_EXIT"
    fi

    # Show only error headers, summaries, and test results.
    grep -E '(^==[0-9]+==(ERROR|WARNING)|SUMMARY:|test result:)' "$TMPFILE" || true
fi

if [[ "$SUPPRESS_MODE" == "1" ]]; then
    # Extract broad suppression patterns from leak stack traces.
    # For each leak block, find the shallowest project-local frame and use its
    # top-level test/module name (e.g. "tcp_connect::" or "compio_quic::").
    REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

    # Get all project-local function names from leak traces.
    # Extract the top-level module::function prefix (first two path segments).
    PATTERNS=$(grep -oP '^\s+#\d+ 0x[0-9a-f]+ in \K\S+(?= '"$REPO_ROOT"')' "$TMPFILE" \
        | sed 's/<[^>]*>//g; s/::{closure#[0-9]*}//g; s/::{shim[^}]*}//g' \
        | awk -F'::' '{if(NF>=2) print $1"::"$2; else print $1}' \
        | sort -u || true)

    if [[ -n "$PATTERNS" ]]; then
        EXISTING=""
        if [[ -f "$SUPP_FILE" ]]; then
            EXISTING=$(grep -oP '^leak:\K.*' "$SUPP_FILE" || true)
        fi

        ADDED=0
        while IFS= read -r pattern; do
            [[ -z "$pattern" ]] && continue
            # Skip if already covered by an existing suppression.
            if echo "$EXISTING" | grep -qxF "$pattern"; then
                continue
            fi
            echo "leak:$pattern" >> "$SUPP_FILE"
            echo "  Added suppression: leak:$pattern"
            ADDED=$((ADDED + 1))
        done <<< "$PATTERNS"

        if [[ "$ADDED" -gt 0 ]]; then
            echo ""
            echo "Added $ADDED new suppression(s) to $SUPP_FILE"
        else
            echo ""
            echo "All detected leaks are already suppressed."
        fi
    else
        echo ""
        echo "No project-local leak frames found to suppress."
    fi
    exit 0
fi

if grep -q 'LeakSanitizer: detected memory leaks' "$TMPFILE"; then
    echo ""
    echo "FAILED: memory leaks detected"
    exit 1
else
    echo ""
    echo "OK: no leaks detected"
fi
