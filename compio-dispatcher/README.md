<div align="center">
    <a href='https://compio.rs'>
        <img height="150" src="https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-with-text.svg">
    </a>
</div>

---

# compio-dispatcher

[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/compio-rs/compio/blob/master/LICENSE)
[![crates.io](https://img.shields.io/crates/v/compio-dispatcher)](https://crates.io/crates/compio-dispatcher)
[![docs.rs](https://img.shields.io/badge/docs.rs-compio--dispatcher-latest)](https://docs.rs/compio-dispatcher)
[![Check](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml)
[![Test](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml)

Multithreading dispatcher for compio.

This crate provides utilities to dispatch tasks across multiple compio runtime threads, enabling parallel processing while maintaining the benefits of compio's thread-per-core model.

## Usage

Use `compio` directly with `dispatcher` feature enabled:

```bash
cargo add compio --features dispatcher
```

Example:

```rust
use compio::dispatcher::Dispatcher;

let dispatcher = Dispatcher::builder().worker_threads(4).build().unwrap();
let result = dispatcher.dispatch(|| async {
    // Your async work here
    42
}).await;
```

Notice that you're dispatching a `Send` closure that returns `Future` to the other threads. The `Future` returned by the closure does not need to be `Send`, which is convenient since lots of the operations are single-thread and not `Send` in `compio`.
