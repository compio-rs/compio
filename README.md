# Compio

A thread-per-core Rust runtime with IOCP/io_uring.
The name comes from "completion-based IO".

## Why not Tokio?

Tokio is a great generic-propose async runtime.
However, it is poll-based, and even uses [undocumented APIs](https://notgull.github.io/device-afd/) on Windows.
We would like some new high-level APIs to perform IOCP/io_uring.

Unlike `tokio-uring`, this runtime isn't Tokio-based.
This is mainly because that no public APIs to control IOCP in `mio`,
and `tokio` won't public APIs to control `mio` before `mio` reaches 1.0.

## Quick start
