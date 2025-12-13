<div align="center">
    <a href='https://compio.rs'>
        <img height="150" src="https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-with-text.svg">
    </a>
</div>

---

# compio-tls

[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/compio-rs/compio/blob/master/LICENSE)
[![crates.io](https://img.shields.io/crates/v/compio-tls)](https://crates.io/crates/compio-tls)
[![docs.rs](https://img.shields.io/badge/docs.rs-compio--tls-latest)](https://docs.rs/compio-tls)
[![Check](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml)
[![Test](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml)

TLS adaptor for compio.

This crate provides TLS/SSL support for compio networking types. It offers both native TLS (using platform-specific implementations) and rustls (pure Rust TLS) backends, allowing you to secure your network connections.

## Features

- TLS client and server support
- Multiple backend options:
  - `native-tls`: Platform-specific TLS (SChannel on Windows, Secure Transport on macOS, OpenSSL on Linux)
  - `rustls`: Pure Rust TLS implementation
- ALPN (Application-Layer Protocol Negotiation) support
- Integration with compio's completion-based IO model

## Usage

Use `compio` directly with `tls` feature and your chosen backend:

```bash
cargo add compio --features tls,rustls # or native-tls
```
