<div align="center">
    <a href='https://compio.rs'>
        <img height="150" src="https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-with-text.svg">
    </a>
</div>

---

# Compio

[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/compio-rs/compio/blob/master/LICENSE)
[![crates.io](https://img.shields.io/crates/v/compio)](https://crates.io/crates/compio)
[![docs.rs](https://img.shields.io/badge/docs.rs-compio-latest)](https://docs.rs/compio)
[![Check](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml)
[![Test](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml)
[![Telegram](https://img.shields.io/badge/Telegram-compio--rs-blue?logo=telegram)](https://t.me/compio_rs)

A thread-per-core Rust runtime with IOCP/io_uring/polling inspired by [monoio](https://github.com/bytedance/monoio).

## Quick start

Add `compio` as dependency:

```bash
cargo add compio --features macros,fs
```

Then use the high level APIs provided to perform filesystem & network IO:

```rust
use compio::{fs::File, io::AsyncReadAtExt};

#[compio::main]
async fn main() {
    let file = File::open("Cargo.toml").await.unwrap();
    let (read, buffer) = file.read_to_end_at(Vec::with_capacity(1024), 0).await.unwrap();
    assert_eq!(read, buffer.len());
    let buffer = String::from_utf8(buffer).unwrap();
    println!("{}", buffer);
}
```

It's also possible to use the low-level driver (the proactor, without async executor) manually. See [`driver` example](./compio/examples/driver.rs).

## Why the name?

The name comes from "completion-based IO", and follows the non-existent convention that an async runtime should be named with a suffix "io".

## Comparison with other runtimes

### Tokio

Tokio is a great generic-purpose async runtime. However, it is poll-based, and even uses [undocumented APIs](https://notgull.net/device-afd/) on Windows. We would like some new high-level APIs to perform IOCP/io_uring.

`compio` isn't Tokio-based. This is mainly because `mio` does not expose any public APIs to control IOCP, and `tokio` won't expose APIs to control `mio` before `mio` reaches 1.0.

### Monoio

Monoio focuses on Linux and io-uring, and fallbacks to `mio` on other platforms.

### Glommio

Glommio doesn't support Windows.

### Others

There are also lots of other great async runtimes. But most of them are (at the moment when `compio` was created) either poll-based, use io-uring [unsoundly](https://without.boats/blog/io-uring/), or aren't cross-platform. We hope `compio` can fill this gap.

## Contributing

There are opportunities to contribute to Compio at any level. It doesn't matter if
you are just getting started with Rust or are the most weathered expert, we can
use your help. If you have any question about Compio, feel free to join our [telegram group](https://t.me/compio_rs).
Before contributing, please checkout our [contributing guide](https://github.com/compio-rs/compio/blob/master/CONTRIBUTING.md).
