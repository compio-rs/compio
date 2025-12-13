<div align="center">
    <a href='https://compio.rs'>
        <img height="150" src="https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-with-text.svg">
    </a>
</div>

---

# compio-net

[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/compio-rs/compio/blob/master/LICENSE)
[![crates.io](https://img.shields.io/crates/v/compio-net)](https://crates.io/crates/compio-net)
[![docs.rs](https://img.shields.io/badge/docs.rs-compio--net-latest)](https://docs.rs/compio-net)
[![Check](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml)
[![Test](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml)

Networking IO for compio.

This crate provides async networking primitives built on compio's completion-based IO model. 

## Usage

Use `compio` directly with `net` feature enabled:

```bash
cargo add compio --features net
```

Example:

```rust
use compio::net::TcpListener;
use compio::io::{AsyncReadExt, AsyncWriteExt};

let listener = TcpListener::bind("127.0.0.1:8080").await?;
loop {
    let (stream, addr) = listener.accept().await?;
    // Handle connection
}
```
