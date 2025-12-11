<div align="center">
    <a href='https://compio.rs'>
        <img height="150" src="https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-with-text.svg">
    </a>
</div>

---

# compio-signal

[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/compio-rs/compio/blob/master/LICENSE)
[![crates.io](https://img.shields.io/crates/v/compio-signal)](https://crates.io/crates/compio-signal)
[![docs.rs](https://img.shields.io/badge/docs.rs-compio--signal-latest)](https://docs.rs/compio-signal)
[![Check](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml)
[![Test](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml)

Signal handling for compio.

This crate provides async signal handling capabilities for compio applications. It allows you to receive and handle OS signals (like SIGINT, SIGTERM on Unix, or Ctrl-C on Windows) in an async context.

## Usage

Use `compio` directly with `signal` feature enabled:

```bash
cargo add compio --features signal
```

Example:

```rust
use compio::signal::ctrl_c;

println!("Waiting for ctrl-c");
let mut sigint = ctrl_c().await?;
println!("ctrl-c received!");
```
