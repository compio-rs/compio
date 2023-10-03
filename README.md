# Compio

[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/Berrysoft/compio/blob/master/LICENSE)
[![crates.io](https://img.shields.io/crates/v/compio)](https://crates.io/crates/compio)
[![docs.rs](https://img.shields.io/badge/docs.rs-compio-latest)](https://docs.rs/compio)
[![Azure DevOps builds](https://strawberry-vs.visualstudio.com/compio/_apis/build/status/Berrysoft.compio?branch=master)](https://strawberry-vs.visualstudio.com/compio/_build)

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

## Why not monoio/tokio-uring?

They don't support Windows.

## Quick start

With `macros` feature enabled, we can use the high level APIs to perform fs & net IO.

```rust
use compio::fs::File;

#[compio::main]
async fn main() {
    let file = File::open("Cargo.toml").unwrap();
    let (read, buffer) = file.read_to_end_at(Vec::with_capacity(1024), 0).await.unwrap();
    assert_eq!(read, buffer.len());
    let buffer = String::from_utf8(buffer).unwrap();
    println!("{}", buffer);
}
```

While you can also control the low-level driver manually. See `driver` example of the repo.
