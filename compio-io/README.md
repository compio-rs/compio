<div align="center">
    <a href='https://compio.rs'>
        <img height="150" src="https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-with-text.svg">
    </a>
</div>

---

# compio-io

[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/compio-rs/compio/blob/master/LICENSE)
[![crates.io](https://img.shields.io/crates/v/compio-io)](https://crates.io/crates/compio-io)
[![docs.rs](https://img.shields.io/badge/docs.rs-compio--io-latest)](https://docs.rs/compio-io)
[![Check](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml)
[![Test](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml)

IO traits for completion-based async IO.

This crate provides async IO traits designed for completion-based operations. Unlike traditional poll-based async IO traits, these traits work with owned buffers and return both the buffer and the operation result upon completion.

The crate itself is runtime-agnostic and can be used with any completion-based async IO runtime.

## Usage

### For application

Use `compio::io` re-exported from `compio` crate, then use the io traits in your application:

```rust
use compio::io::{AsyncRead, AsyncWrite};
```

### For library

If you are writing libraries that want to support completion-based async IO, you can depend on this crate directly:

```bash
cargo add compio-io
```

Then you can use the io traits in your library:

```rust
use compio_io::{AsyncRead, AsyncWrite};
```

## Content
### Fundamental

- `AsyncRead`: Async read into a buffer implements `IoBufMut`
- `AsyncReadAt`: Async read into a buffer implements `IoBufMut` with
  offset
- `AsyncWrite`: Async write from a buffer implements `IoBuf`
- `AsyncWriteAt`: Async write from a buffer implements `IoBuf` with
  offset

### Buffered IO

- `AsyncBufRead`: Trait of async read with buffered content
- `BufReader`: An async reader with internal buffer
- `BufWriter`: An async writer with internal buffer

### Extension

- `AsyncReadExt`: Extension trait for `AsyncRead`
- `AsyncReadAtExt`: Extension trait for `AsyncReadAt`
- `AsyncWriteExt`: Extension trait for `AsyncWrite`
- `AsyncWriteAtExt`: Extension trait for `AsyncWriteAt`

### Adapters
- `framed::Framed`: Adapts `AsyncRead` to `Stream` and `AsyncWrite`
  to `Sink`, with framed en/decoding.
- `compat::SyncStream`: Adapts async IO to std blocking io (requires
  `compat` feature)
- `compat::AsyncStream`: Adapts async IO to `futures_util::io` traits
  (requires `compat` feature)
  
### Utils
See docs.rs for detail: `copy`, `null`, `repeat`, `split`, `split_unsync`.
