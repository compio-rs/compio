<div align="center">
    <a href='https://compio.rs'>
        <img height="150" src="https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-with-text.svg">
    </a>
</div>

---

# compio-process

[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/compio-rs/compio/blob/master/LICENSE)
[![crates.io](https://img.shields.io/crates/v/compio-process)](https://crates.io/crates/compio-process)
[![docs.rs](https://img.shields.io/badge/docs.rs-compio--process-latest)](https://docs.rs/compio-process)
[![Check](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml)
[![Test](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml)

Process management for compio.

This crate provides async process spawning and management capabilities for compio applications. It allows you to spawn child processes, interact with their stdio streams, and wait for completion asynchronously.

## Features

- Async process spawning and management
- Async stdio (stdin, stdout, stderr) access
- Cross-platform support (Unix and Windows)
- Integration with compio's completion-based IO model
- Optional Linux pidfd support for efficient process monitoring

## Usage

Use `compio` directly with `process` feature enabled:

```bash
cargo add compio --features process
```

Example:

```rust
use compio::process::Command;

let mut child = Command::new("echo")
    .arg("Hello from compio!")
    .spawn()?;

let status = child.wait().await?;
println!("Process exited with: {}", status);
```
