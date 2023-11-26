# Compio

[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/compio-rs/compio/blob/master/LICENSE)
[![crates.io](https://img.shields.io/crates/v/compio)](https://crates.io/crates/compio)
[![docs.rs](https://img.shields.io/badge/docs.rs-compio-latest)](https://docs.rs/compio)
[![Azure DevOps builds](https://strawberry-vs.visualstudio.com/compio/_apis/build/status/compio-rs.compio?branch=master)](https://strawberry-vs.visualstudio.com/compio/_build?definitionId=22)
[![Telegram](https://img.shields.io/badge/Telegram-compio--rs-blue?logo=telegram)](https://t.me/compio_rs)

A thread-per-core Rust runtime with IOCP/io_uring/polling.
The name comes from "completion-based IO".
This crate is inspired by [monoio](https://github.com/bytedance/monoio/).

## Why not Tokio?

Tokio is a great generic-propose async runtime.
However, it is poll-based, and even uses [undocumented APIs](https://notgull.net/device-afd/) on Windows.
We would like some new high-level APIs to perform IOCP/io_uring.

Unlike `tokio-uring`, this runtime isn't Tokio-based.
This is mainly because that no public APIs to control IOCP in `mio`,
and `tokio` won't public APIs to control `mio` before `mio` reaches 1.0.

## Why not monoio/tokio-uring/glommio?

They don't support Windows.

## Quick start

Add `compio` as dependency:

```
compio = { version = "0.9.0-beta.1", features = ["macros"] }
```

Then we can use high level APIs to perform filesystem & net IO.

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

You can also control the low-level driver manually. See `driver` example of the repo.

## Contributing

There are opportunities to contribute to Compio at any level. It doesn't matter if
you are just getting started with Rust or are the most weathered expert, we can
use your help. If you have any question about Compio, feel free to join our [telegram group](https://t.me/compio_rs). Before contributing, please checkout our [contributing guide](https://github.com/compio-rs/compio/blob/master/CONTRIBUTING.md).
