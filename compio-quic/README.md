<div align="center">
    <a href='https://compio.rs'>
        <img height="150" src="https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-with-text.svg">
    </a>
</div>

---

# compio-quic

[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/compio-rs/compio/blob/master/LICENSE)
[![crates.io](https://img.shields.io/crates/v/compio-quic)](https://crates.io/crates/compio-quic)
[![docs.rs](https://img.shields.io/badge/docs.rs-compio--quic-latest)](https://docs.rs/compio-quic)
[![Check](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml)
[![Test](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml)

QUIC protocol implementation for compio.

This crate provides QUIC (Quick UDP Internet Connections) support for compio, built on top of quinn-proto. QUIC is a modern transport protocol that provides features like multiplexing, built-in encryption, and improved connection migration, making it ideal for applications like HTTP/3.

## Features

- QUIC client and server support
- Built on quinn-proto for robust QUIC implementation
- Optional HTTP/3 support via the `h3` feature
- Multiple certificate verification options:
  - `platform-verifier`: Use platform-specific certificate verification
  - `native-certs`: Use system's native certificate store
  - `webpki-roots`: Use Mozilla's root certificates
- Integration with compio's completion-based IO model
- Cross-platform support

## Usage

Use `compio` directly with `quic` feature enabled:

```bash
cargo add compio --features quic
```

Example:

```rust
use compio::quic::{Endpoint, ClientConfig};

let mut endpoint = Endpoint::client("0.0.0.0:0".parse()?)?;
let connection = endpoint.connect("compio.rs:443", "compio.rs").await?;

// Use the QUIC connection
let (mut send, mut recv) = connection.open_bi().await?;
send.write_all(b"Hello QUIC!").await?;
```
