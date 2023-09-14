# Compio

[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/Berrysoft/compio/blob/master/LICENSE)
[![crates.io](https://img.shields.io/crates/v/compio)](https://crates.io/crates/compio)
[![docs.rs](https://img.shields.io/badge/docs.rs-compio-latest)](https://docs.rs/compio)
[![Azure DevOps builds](https://strawberry-vs.visualstudio.com/compio/_apis/build/status/Berrysoft.compio?branch=master)](https://strawberry-vs.visualstudio.com/compio/_build)

A thread-per-core Rust runtime with IOCP/io_uring/mio.
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

With `runtime` feature enabled, we can use the high level APIs to perform fs & net IO.

```rust,no_run
use compio::{fs::File, task::block_on};

let buffer = block_on(async {
    let file = File::open("Cargo.toml").unwrap();
    let (read, buffer) = file.read_at(Vec::with_capacity(1024), 0).await;
    let read = read.unwrap();
    assert_eq!(read, buffer.len());
    String::from_utf8(buffer).unwrap()
});
println!("{}", buffer);
```

While you can also control the low-level driver manually:

```rust,no_run
use arrayvec::ArrayVec;
use compio::{
    buf::IntoInner,
    driver::{AsRawFd, Driver, Entry, Poller},
    fs::File,
    op::ReadAt,
};

let mut driver = Driver::new().unwrap();
let file = File::open("Cargo.toml").unwrap();
// Attach the `RawFd` to driver first.
driver.attach(file.as_raw_fd()).unwrap();

// Create operation and push it to the driver.
let mut op = ReadAt::new(file.as_raw_fd(), 0, Vec::with_capacity(4096));
let ops = [(&mut op, 0).into()];

// Poll the driver and wait for IO completed.
let mut entries = ArrayVec::<Entry, 1>::new();
unsafe {
    driver
        .poll(None, &mut ops.into_iter(), &mut entries)
        .unwrap();
}
let entry = entries.drain(..).next().unwrap();
assert_eq!(entry.user_data(), 0);

// Resize the buffer by return value.
let n = entry.into_result().unwrap();
let mut buffer = op.into_inner().into_inner();
unsafe {
    buffer.set_len(n);
}

println!("{}", String::from_utf8(buffer).unwrap());
```
