<div align="center">
    <a href='https://compio.rs'>
        <img height="150" src="https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-with-text.svg">
    </a>
</div>

---

# compio-macros

[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/compio-rs/compio/blob/master/LICENSE)
[![crates.io](https://img.shields.io/crates/v/compio-macros)](https://crates.io/crates/compio-macros)
[![docs.rs](https://img.shields.io/badge/docs.rs-compio--macros-latest)](https://docs.rs/compio-macros)
[![Check](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml)
[![Test](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml)

Procedural macros for compio.

This crate provides convenience macros for working with the compio runtime, making it easier to write async applications.

## Macros

- `#[compio::main]` - Marks an async function as the entry point, setting up the compio runtime
- `#[compio::test]` - Marks an async function as a test, running it on a compio runtime

## Usage

Use `compio::macros` re-exported from `compio` crate, then apply the macros in your application:

```bash
cargo add compio --features macros
```

Example:

```rust
use compio_macros::main;

#[compio::main]
async fn main() {
    println!("Hello from compio!");
}
```
