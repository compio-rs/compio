<div align="center">
    <a href='https://compio.rs'>
        <img height="150" src="https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-with-text.svg">
    </a>
</div>

---

# compio-runtime

[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/compio-rs/compio/blob/master/LICENSE)
[![crates.io](https://img.shields.io/crates/v/compio-runtime)](https://crates.io/crates/compio-runtime)
[![docs.rs](https://img.shields.io/badge/docs.rs-compio--runtime-latest)](https://docs.rs/compio-runtime)
[![Check](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml)
[![Test](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml)

High-level runtime for compio.

This crate provides the async runtime that coordinates task execution with the low-level driver. It implements a thread-per-core model with work-stealing task scheduling, providing an efficient foundation for building async applications.

## Features

- Thread-per-core architecture with CPU affinity
- Work-stealing task scheduler
- Optional time/timer support
- Optional event notification primitives
- Integration with compio-driver for IO operations

## Usage

Add this to your `Cargo.toml`:

```toml
[dependencies]
compio-runtime = "0.10"
```

Example:

```rust
use compio_runtime::Runtime;

let runtime = Runtime::new().unwrap();
runtime.block_on(async {
    // Your async code here
});
```

Most users will use this crate indirectly through the main `compio` crate with the `runtime` feature enabled.