<div align="center">
    <a href='https://compio.rs'>
        <img height="150" src="https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-with-text.svg">
    </a>
</div>

---

# compio-buf

[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/compio-rs/compio/blob/master/LICENSE)
[![crates.io](https://img.shields.io/crates/v/compio-buf)](https://crates.io/crates/compio-buf)
[![docs.rs](https://img.shields.io/badge/docs.rs-compio--buf-latest)](https://docs.rs/compio-buf)
[![Check](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml)
[![Test](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml)

Buffer traits for completion-based async IO.

This crate provides fundamental buffer traits (`IoBuf`, `IoBufMut`, `IoVectoredBuf`, `IoVectoredBufMut`) to interact with compio (and other runtimes). The crate itself is runtime-agnostic and can be used anywhere.

## Usage

### For application

Use `compio::buf` re-exported from `compio` crate, then use the buffer traits in your application:

```rust
use compio::buf::{IoBuf, IoBufMut};
```

### For library

If you are writing libraries that want to support compio-buf, you can depend on this crate directly:

```bash
cargo add compio-buf
```

Then you can use the buffer traits in your library:

```rust
use compio_buf::{IoBuf, IoBufMut};
```
