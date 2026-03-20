# External Testing Suite

External (out-of-process) tests for compio-h2. These complement the
Rust unit and integration tests by exercising the server from independent HTTP/2 clients.

## What's Covered

| Suite | Language | Purpose |
|-------|----------|---------|
| **h2spec** | Go binary | RFC 9113 + RFC 7541 conformance (147 tests) |
| **h2load** | C (nghttp2) | Load testing — throughput, latency, concurrent streams |
| **cargo-fuzz** | Rust (nightly) | Fuzz testing for frame decode, HPACK decode, frame header |
| **Soak test** | Rust | Long-running RSS monitoring for memory leak detection |

> **Note:** CVE regression tests, malformed-frame edge cases, and protocol
> violation tests (previously Go/Python scripts) are now Rust integration tests
> in `cargo test -p compio-h2 --test security` (30 tests).

## Prerequisites

- **Rust** (stable + nightly for fuzzing and memory safety)
- **nghttp2** (`h2load`) - install via `bash compio-h2/scripts/conformance/install-nghttp2.sh`
- **h2spec** - install via `bash compio-h2/scripts/conformance/install-h2spec.sh`
- **cargo-fuzz** (nightly) - `cargo install cargo-fuzz`

## Quick Start

Run all external test suites:

```bash
bash compio-h2/scripts/conformance/run-all.sh
```

This builds the h2-server example, starts it, and runs each suite sequentially.
Suites whose prerequisites are missing are skipped gracefully.

## Individual Suites

### h2spec Conformance

```bash
bash compio-h2/scripts/conformance/run-h2spec.sh
```

### h2load Load Testing

```bash
bash compio-h2/scripts/conformance/run-h2load.sh
```

### Cargo Fuzz (Nightly)

```bash
cd compio-h2
cargo +nightly fuzz run fuzz_frame_decode -- -max_total_time=60
cargo +nightly fuzz run fuzz_hpack_decode -- -max_total_time=60
cargo +nightly fuzz run fuzz_frame_header -- -max_total_time=60
```

### Memory Safety

```bash
bash compio-h2/scripts/memleak/soak.sh        # RSS soak test
bash compio-h2/scripts/memleak/soak.sh --asan # Soak + ASan
```

## Environment Variables

### Conformance

| Variable | Default | Used By | Description |
|----------|---------|---------|-------------|
| `H2_PORT` | `8080` | All suites | Port for the test server |
| `H2SPEC` | `h2spec` | h2spec | Path to h2spec binary |
| `H2_STARTUP_TIMEOUT` | `10` | lib.sh | Seconds to wait for server startup |
| `H2LOAD_REQUESTS` | `10000` | h2load | Total requests to send |
| `H2LOAD_CLIENTS` | `10` | h2load | Number of concurrent clients |
| `H2LOAD_STREAMS` | `100` | h2load | Max concurrent streams per client |

### Soak Test

| Variable | Default | Description |
|----------|---------|-------------|
| `SOAK_REQUESTS` | `100000` | Total requests to send |
| `SOAK_PAYLOAD_SIZE` | `1024` | Request body size in bytes |
| `SOAK_BATCH_SIZE` | `1000` | Requests per progress report |
| `SOAK_RECONNECT_EVERY` | `0` | Reconnect after N requests (0 = never) |
| `SOAK_MAX_RSS_GROWTH_KB` | `512` | Fail if RSS grows more than this |

## Shared Helpers (lib.sh)

All `run-*.sh` scripts source `conformance/lib.sh`, which provides:

- `build_test_server` - builds the `h2-server` example binary
- `start_test_server PORT` - starts the server and registers a cleanup trap
- `wait_for_server PORT [TIMEOUT]` - polls until the server accepts TCP connections
- `stop_test_server` - graceful shutdown with SIGKILL fallback

## Directory Structure

```
compio-h2/
├── fuzz/                       # cargo-fuzz targets
│   ├── Cargo.toml
│   └── fuzz_targets/
│       ├── fuzz_frame_decode.rs
│       ├── fuzz_hpack_decode.rs
│       └── fuzz_frame_header.rs
└── scripts/
    ├── README.md               # This file
    ├── conformance/            # H2 spec conformance (h2spec, h2load)
    │   ├── lib.sh              # Shared helpers (build/start/stop server)
    │   ├── install-h2spec.sh
    │   ├── install-nghttp2.sh
    │   ├── run-all.sh          # Master orchestration
    │   ├── run-h2spec.sh
    │   └── run-h2load.sh
    └── memleak/                # Memory safety testing
        └── soak.sh             # RSS soak test (--asan flag for ASan mode)
```
