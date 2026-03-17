#!/usr/bin/env bash
# Run the H2 soak test to detect memory leaks via RSS monitoring.
#
# Usage:
#   ./scripts/memleak/soak.sh           # release build, 50K requests
#   ./scripts/memleak/soak.sh --asan    # with AddressSanitizer

set -euo pipefail

cd "$(dirname "$0")/../../.."  # workspace root

if [[ "${1:-}" == "--asan" ]]; then
    shift
    echo "=== H2 Soak Test (ASan) ==="
    TARGET=$(rustc -vV | grep host | awk '{print $2}')
    export RUSTFLAGS="${RUSTFLAGS:-} -Zsanitizer=address"
    export ASAN_OPTIONS="${ASAN_OPTIONS:-detect_leaks=1:halt_on_error=0}"
    cargo +nightly run -Zbuild-std --target "$TARGET" --release -p compio-h2 --example soak_h2 "$@"
else
    echo "=== H2 Soak Test ==="
    cargo run --release -p compio-h2 --example soak_h2 "$@"
fi
