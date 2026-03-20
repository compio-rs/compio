<div align="center">
    <a href='https://compio.rs'>
        <img height="150" src="https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-with-text.svg">
    </a>
</div>

---

# compio-h2

[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/compio-rs/compio/blob/master/LICENSE)
[![crates.io](https://img.shields.io/crates/v/compio-h2)](https://crates.io/crates/compio-h2)
[![docs.rs](https://img.shields.io/badge/docs.rs-compio--h2-latest)](https://docs.rs/compio-h2)
[![Check](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml)
[![Test](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml)

HTTP/2 protocol implementation for compio.

This crate provides a native HTTP/2 client and server built on compio's completion-based I/O, without depending on tokio or the `h2` crate. It implements the full HTTP/2 wire protocol including HPACK header compression (RFC 7541), stream multiplexing, flow control, and connection management per RFC 9113.

## Features

- Native HTTP/2 implementation (client + server) for the compio completion-based async runtime
- Single-threaded design: no `Send`/`Sync` bounds, built for compio's thread-per-core model
- Full HTTP/2 frame codec (DATA, HEADERS, CONTINUATION, SETTINGS, PING, GOAWAY, RST\_STREAM, PRIORITY, WINDOW\_UPDATE)
- HPACK header compression with build.rs-generated Huffman decode table and O(1) hash-based dynamic table lookup
- Per-stream and connection-level flow control with send-side backpressure
- `ClientBuilder` / `ServerBuilder` with fluent configuration for all HTTP/2 settings, keepalive, and buffer limits
- Configurable PING/PONG keepalive with timeout detection
- Two-phase GOAWAY graceful shutdown with stream draining
- RST\_STREAM flood protection (CVE-2023-44487 mitigation) with configurable rate limits
- Structured error types with reason code extraction and classification helpers
- Optional TLS via the `tls` feature (compio-tls + rustls, ALPN h2)
- Passes all 147/147 [h2spec](https://github.com/summerwind/h2spec) tests in strict mode

## Usage

Use `compio` directly with `h2` feature enabled:

```bash
cargo add compio --features h2
```

Or add `compio-h2` directly:

```bash
cargo add compio-h2
```

Example:

```rust
use compio_h2::ClientBuilder;
use compio_net::TcpStream;

let tcp = TcpStream::connect("127.0.0.1:8080").await?;
let (mut send_request, conn) = ClientBuilder::new().handshake(tcp).await?;
compio_runtime::spawn(conn.run()).detach();

let request = http::Request::builder()
    .uri("http://127.0.0.1:8080/")
    .body(())
    .unwrap();
let (response, _send_stream) = send_request.send_request(request, true)?;
let response = response.await_response().await?;
println!("Response: {:?}", response.status());
```
