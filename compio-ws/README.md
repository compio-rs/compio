<div align="center">
    <a href='https://compio.rs'>
        <img height="150" src="https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-with-text.svg">
    </a>
</div>

---

# compio-ws

[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/compio-rs/compio/blob/master/LICENSE)
[![crates.io](https://img.shields.io/crates/v/compio-ws)](https://crates.io/crates/compio-ws)
[![docs.rs](https://img.shields.io/badge/docs.rs-compio--ws-latest)](https://docs.rs/compio-ws)
[![Check](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml)
[![Test](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml)

WebSocket library for compio.

This crate provides WebSocket client and server support for compio applications, built on top of the tungstenite WebSocket library. It enables real-time bidirectional communication over TCP connections with optional TLS support.

## Features

- WebSocket client and server support
- Built on tungstenite for robust WebSocket implementation
- TLS/SSL support with multiple backends:
  - `native-tls`: Platform-specific TLS
  - `rustls`: Pure Rust TLS implementation
- Certificate verification options (platform-verifier, native-certs, webpki-roots)
- Integration with compio's completion-based IO model
- Support for both text and binary messages

## Usage

Add this to your `Cargo.toml`:

```toml
[dependencies]
compio-ws = "0.2"
```

For secure WebSocket connections (wss://), enable a TLS backend:

```toml
[dependencies]
compio-ws = { version = "0.2", features = ["rustls"] }
```

Example:

```rust
use compio_ws::connect_async;

let (mut ws_stream, _) = connect_async("ws://example.com/socket").await?;

// Send and receive messages
ws_stream.send(Message::text("Hello WebSocket!")).await?;
let msg = ws_stream.next().await.unwrap()?;
```

This crate is available through the main `compio` crate with the `ws` feature.