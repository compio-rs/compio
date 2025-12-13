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

This crate provides the async runtime (executor) that coordinates task execution with the low-level driver (proactor). It implements a thread-per-core model.

## Usage

The recommended way is to use `main` macro with `compio`'s `macros` feature, but you can also use the runtime directly by enabling the `runtime` feature:

```bash
cargo add compio --features runtime
```

Example:

```rust
use compio::runtime::Runtime;

let runtime = Runtime::new().unwrap();
runtime.block_on(async {
    // Your async code here
});
```

## Configuration

The runtime can be configured using the `RuntimeBuilder`:

```rust
use compio::runtime::RuntimeBuilder;
use compio::driver::ProactorBuilder;

let mut proactor = ProactorBuilder::new();

// Configure proactor here, e.g.
proactor.capacity(1024);
    
let runtime = RuntimeBuilder::new()
    .with_proactor(proactor)
    // Configure other options here
    .build()
    .unwrap();

runtime.block_on(async {
    // Your async code here
});
```
