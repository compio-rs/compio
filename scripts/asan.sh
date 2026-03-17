#!/usr/bin/env bash
# Run workspace tests with AddressSanitizer (ASan) + LeakSanitizer (LSan) to
# detect memory leaks, use-after-free, buffer overflows, and stack overflows.
#
# Suppressions are defined in scripts/asan.toml, scoped per crate and test file.
# The script builds the entire workspace (preserving feature unification), then
# runs each test binary individually with only its crate's suppressions active.
#
# Requires: nightly toolchain with rust-src component
#   rustup component add rust-src --toolchain nightly
#
# Usage:
#   ./scripts/asan.sh <target>                # run all tests
#   ./scripts/asan.sh --ci <target>           # CI mode: test failures don't affect exit code
#   ./scripts/asan.sh --verbose <target>      # show full test + ASan output
#   ./scripts/asan.sh --suppress <target>     # run and print new leak patterns per crate
#   ./scripts/asan.sh <target> -- basic       # filter by test name

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TOML_FILE="$SCRIPT_DIR/asan.toml"
SUPPRESS_MODE=0
CI_MODE=0

# Parse flags.
while [[ "${1:-}" == --* ]]; do
    case "$1" in
        --suppress) SUPPRESS_MODE=1; shift ;;
        --ci)       CI_MODE=1; shift ;;
        --verbose)  export ASAN_VERBOSE=1; shift ;;
        *)          break ;;
    esac
done

TARGET="${1:?Usage: $0 [--ci] [--suppress] <target-triple> [-- test-filter]}"
shift

# Collect test-name filter args (everything after --).
TEST_ARGS=()
if [[ "${1:-}" == "--" ]]; then
    shift
    TEST_ARGS=("$@")
fi

# ASan + LSan: detect memory errors and leaks at process exit.
export RUSTFLAGS="${RUSTFLAGS:-} -Zsanitizer=address"
export ASAN_OPTIONS="${ASAN_OPTIONS:-detect_leaks=1:halt_on_error=0}"

# Symbolizer is required for LSan suppressions (they match on function names).
if [[ -z "${ASAN_SYMBOLIZER_PATH:-}" ]]; then
    for candidate in llvm-symbolizer llvm-symbolizer-{20,19,18,17,16,15}; do
        if command -v "$candidate" &>/dev/null; then
            export ASAN_SYMBOLIZER_PATH="$(command -v "$candidate")"
            break
        fi
    done
fi

# --- Helper: extract suppression patterns for a crate from asan.toml ---
# Writes leak:pattern lines to stdout.
extract_patterns_for_crate() {
    local crate="$1"
    [[ -f "$TOML_FILE" ]] || return 0
    python3 -c "
import sys, re
crate, toml_path = sys.argv[1], sys.argv[2]
with open(toml_path) as f:
    content = f.read()
in_crate = in_patterns = False
for line in content.splitlines():
    s = line.strip()
    if s.startswith('#'):
        continue
    m = re.match(r'^\[(.+)\]$', s)
    if m:
        parts = m.group(1).split('.', 1)
        in_crate = (parts[0].strip().strip('\"') == crate)
        in_patterns = False
        continue
    if not in_crate:
        continue
    if 'patterns' in s and '=' in s:
        in_patterns = True
    if in_patterns:
        for p in re.findall(r'\"([^\"]+)\"', s):
            if not p.startswith('tests/'):
                print(f'leak:{p}')
        if ']' in s:
            in_patterns = False
" "$crate" "$TOML_FILE"
}

echo "=== ASan + LSan: detecting memory errors and leaks ==="
echo "Target: $TARGET"
if [[ "$CI_MODE" == "1" ]]; then
    echo "Mode: CI (test failures ignored)"
fi
echo ""

# --- Phase 1: Build all test binaries (workspace feature unification) ---
echo "Building workspace with ASan ..."
BUILD_JSON=$(mktemp)
CARGO_EXIT=0
# -Zbuild-std instruments std with ASan (prevents false positives and
# compilation errors in deps like zerocopy that depend on std internals).
cargo +nightly test -Zbuild-std --target "$TARGET" --workspace --lib --tests \
    --no-run --message-format=json 2>/dev/null > "$BUILD_JSON" || CARGO_EXIT=$?

if [[ "$CARGO_EXIT" -ne 0 ]]; then
    # Show human-readable errors by re-running without JSON.
    cargo +nightly test -Zbuild-std --target "$TARGET" --workspace --lib --tests --no-run 2>&1 || true
    echo ""
    echo "FAILED: build failed (exit $CARGO_EXIT)"
    rm -f "$BUILD_JSON"
    exit "$CARGO_EXIT"
fi

# Parse test binaries from cargo JSON output: crate<TAB>name<TAB>path
# Sorted by crate name so headers only print once.
BINARIES=$(python3 -c "
import sys, json
bins = []
for line in open(sys.argv[1]):
    try: msg = json.loads(line)
    except: continue
    if msg.get('reason') != 'compiler-artifact': continue
    if not msg.get('profile', {}).get('test'): continue
    exe = msg.get('executable', '')
    if not exe: continue
    pkg_id = msg.get('package_id', '')
    if '/' in pkg_id:
        pkg = pkg_id.split('#')[0].split('/')[-1]
    else:
        pkg = pkg_id.split(' ')[0]
    name = msg.get('target', {}).get('name', '')
    bins.append((pkg, name, exe))
bins.sort(key=lambda x: x[0])
for pkg, name, exe in bins:
    print(f'{pkg}\t{name}\t{exe}')
" "$BUILD_JSON")
rm -f "$BUILD_JSON"

BINARY_COUNT=$(echo "$BINARIES" | wc -l)
echo "Built $BINARY_COUNT test binaries."
echo ""

# --- Phase 2: Run each binary with per-crate suppressions ---
LEAK_DETAILS=()
FAIL_DETAILS=()
TMPFILES=()
trap 'rm -f "${TMPFILES[@]}"' EXIT

# Group binaries by crate, generate one supp file per crate.
PREV_CRATE=""
CRATE_SUPP=""
BIN_INDEX=0

while IFS=$'\t' read -r CRATE NAME EXE; do
    BIN_INDEX=$((BIN_INDEX + 1))

    # Generate suppression file when we encounter a new crate.
    if [[ "$CRATE" != "$PREV_CRATE" ]]; then
        SUPP_TMP=$(mktemp)
        TMPFILES+=("$SUPP_TMP")
        if [[ "$SUPPRESS_MODE" != "1" ]]; then
            extract_patterns_for_crate "$CRATE" > "$SUPP_TMP"
        fi
        CRATE_SUPP="$SUPP_TMP"

        # Print crate header.
        if [[ "$SUPPRESS_MODE" != "1" ]]; then
            if [[ -s "$CRATE_SUPP" ]]; then
                SUPP_COUNT=$(wc -l < "$CRATE_SUPP")
                SUPP_LIST=$(sed 's/^leak://' "$CRATE_SUPP" | head -3 | paste -sd', ')
                if [[ "$SUPP_COUNT" -gt 3 ]]; then
                    SUPP_LIST="$SUPP_LIST, ... ($SUPP_COUNT total)"
                fi
                echo "--- $CRATE ($SUPP_COUNT suppressed: $SUPP_LIST) ---"
            else
                echo "--- $CRATE ---"
            fi
        else
            echo "--- $CRATE ---"
        fi
        PREV_CRATE="$CRATE"
    fi

    # Set per-crate suppressions.
    if [[ "$SUPPRESS_MODE" == "1" ]]; then
        export LSAN_OPTIONS=""
    elif [[ -s "$CRATE_SUPP" ]]; then
        export LSAN_OPTIONS="suppressions=$CRATE_SUPP"
    else
        export LSAN_OPTIONS=""
    fi

    TMPFILE=$(mktemp)
    TMPFILES+=("$TMPFILE")

    # Run the test binary.
    if [[ "${ASAN_VERBOSE:-0}" == "1" ]]; then
        "$EXE" --test-threads=1 "${TEST_ARGS[@]}" 2>&1 | tee "$TMPFILE"
    else
        "$EXE" --test-threads=1 "${TEST_ARGS[@]}" > "$TMPFILE" 2>&1 || true

        # Parse results.
        RESULT_LINE=$(grep '^test result:' "$TMPFILE" | tail -1 || true)
        if [[ -z "$RESULT_LINE" ]]; then
            TESTS_RAN=0
        else
            P=$(echo "$RESULT_LINE" | grep -oP '\d+(?= passed)' || echo 0)
            F=$(echo "$RESULT_LINE" | grep -oP '\d+(?= failed)' || echo 0)
            I=$(echo "$RESULT_LINE" | grep -oP '\d+(?= ignored)' || echo 0)
            TESTS_RAN=$((P + F))
        fi

        HAS_LEAK=0
        if grep -q 'LeakSanitizer: detected memory leaks' "$TMPFILE"; then
            HAS_LEAK=1
        fi

        # Only print if there's something interesting.
        if [[ "$TESTS_RAN" -gt 0 ]]; then
            if [[ "$HAS_LEAK" -eq 1 ]]; then
                STATUS="LEAK"
            elif [[ "$F" -gt 0 ]]; then
                STATUS="FAIL"
            else
                STATUS="ok"
            fi
            echo "  $NAME: $STATUS ($P passed, $F failed, $I ignored)"
        fi

        # Collect failed test names with panic location for summary.
        if [[ "${TESTS_RAN:-0}" -gt 0 && "${F:-0}" -gt 0 && "$HAS_LEAK" -eq 0 ]]; then
            FAILED_TESTS=$(grep '^test .* FAILED$' "$TMPFILE" | sed 's/^test \(.*\) \.\.\..*FAILED$/\1/' || true)
            while IFS= read -r t; do
                [[ -z "$t" ]] && continue
                # Find the panic location for this test (thread 'test_name' panicked at ...).
                PANIC_LOC=$(grep -A1 "thread '${t}'" "$TMPFILE" | grep -oP "panicked at \K[^,]+" | head -1 || true)
                if [[ -n "$PANIC_LOC" ]]; then
                    FAIL_DETAILS+=("$CRATE/$NAME::$t  ($PANIC_LOC)")
                else
                    FAIL_DETAILS+=("$CRATE/$NAME::$t")
                fi
            done <<< "$FAILED_TESTS"
        fi
    fi

    if [[ "$SUPPRESS_MODE" == "1" ]]; then
        PATTERNS=$(grep -oP '^\s+#\d+ 0x[0-9a-f]+ in \K\S+(?= '"$REPO_ROOT"')' "$TMPFILE" \
            | sed 's/<[^>]*>//g; s/::{closure#[0-9]*}//g; s/::{shim[^}]*}//g' \
            | awk -F'::' '{if(NF>=2) print $1"::"$2; else print $1}' \
            | sort -u || true)

        if [[ -n "$PATTERNS" ]]; then
            while IFS= read -r pattern; do
                [[ -z "$pattern" ]] && continue
                echo "    leak:$pattern"
            done <<< "$PATTERNS"
        fi
    else
        if grep -q 'LeakSanitizer: detected memory leaks' "$TMPFILE"; then
            # Collect leak summary: extract SUMMARY lines and project-local frames.
            SUMMARY_LINE=$(grep 'SUMMARY: AddressSanitizer:' "$TMPFILE" | head -1 || true)
            LEAK_FRAMES=$(grep -oP '^\s+#\d+ 0x[0-9a-f]+ in \K\S+(?= '"$REPO_ROOT"')' "$TMPFILE" \
                | sed 's/<[^>]*>//g; s/::{closure#[0-9]*}//g; s/::{shim[^}]*}//g' \
                | awk -F'::' '{if(NF>=2) print $1"::"$2; else print $1}' \
                | sort -u || true)
            # Get the deepest project-local frame with file:line for each leak block.
            LEAK_LOCS=$(grep -P '^\s+#\d+ 0x[0-9a-f]+ in .+ '"$REPO_ROOT"'/' "$TMPFILE" \
                | sed 's|.*in \(.*\) '"$REPO_ROOT"'/\(.*\)|\2: \1|' \
                | sed 's/<[^>]*>//g; s/::{closure#[0-9]*}//g; s/::{shim[^}]*}//g' \
                | sort -u | head -5 || true)
            LEAK_DETAILS+=("$CRATE/$NAME: ${SUMMARY_LINE#*SUMMARY: }")
            while IFS= read -r loc; do
                [[ -z "$loc" ]] && continue
                LEAK_DETAILS+=("  $loc")
            done <<< "$LEAK_LOCS"
        fi
    fi
done <<< "$BINARIES"

# --- Phase 3: Summary ---
echo ""
if [[ "$SUPPRESS_MODE" == "1" ]]; then
    echo "Suppress mode complete. Add patterns above to scripts/asan.toml under the appropriate crate."
    exit 0
fi

EXIT_CODE=0

# Leak summary.
if [[ "${#LEAK_DETAILS[@]}" -gt 0 ]]; then
    echo "Unsuppressed leaks:"
    for detail in "${LEAK_DETAILS[@]}"; do
        echo "  $detail"
    done
    echo ""
    EXIT_CODE=1
fi

# Failure summary.
if [[ "${#FAIL_DETAILS[@]}" -gt 0 ]]; then
    if [[ "$CI_MODE" == "1" ]]; then
        echo "Test failures (ignored due to --ci mode):"
    else
        echo "Test failures:"
        EXIT_CODE=1
    fi
    for detail in "${FAIL_DETAILS[@]}"; do
        echo "  $detail"
    done
    echo ""
fi

# Final verdict.
HAS_LEAKS=$([[ "${#LEAK_DETAILS[@]}" -gt 0 ]] && echo 1 || echo 0)
HAS_FAILS=$([[ "${#FAIL_DETAILS[@]}" -gt 0 && "$CI_MODE" != "1" ]] && echo 1 || echo 0)

if [[ "$EXIT_CODE" -eq 0 ]]; then
    echo "OK: no leaks detected"
else
    REASONS=()
    [[ "$HAS_LEAKS" == "1" ]] && REASONS+=("memory leaks")
    [[ "$HAS_FAILS" == "1" ]] && REASONS+=("test failures")
    echo "FAILED: $(IFS=' and '; echo "${REASONS[*]}") detected"
fi

exit "$EXIT_CODE"
