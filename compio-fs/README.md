<div align="center">
    <a href='https://compio.rs'>
        <img height="150" src="https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-with-text.svg">
    </a>
</div>

---

# compio-fs

[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/compio-rs/compio/blob/master/LICENSE)
[![crates.io](https://img.shields.io/crates/v/compio-fs)](https://crates.io/crates/compio-fs)
[![docs.rs](https://img.shields.io/badge/docs.rs-compio--fs-latest)](https://docs.rs/compio-fs)
[![Check](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml)
[![Test](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml)

Filesystem IO for compio.

## Usage

Use `compio` directly with `fs` feature enabled:

```bash
cargo add compio --features fs
```

Example:

```rust
use compio::fs::File;
use compio::io::AsyncReadAtExt;

let file = File::open("example.txt").await?;
let (n_read, buffer) = file.read_to_end_at(Vec::new(), 0).await?;
```
