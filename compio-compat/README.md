<div align="center">
    <a href='https://compio.rs'>
        <img height="150" src="https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-with-text.svg">
    </a>
</div>

---

# compio-compat

[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/compio-rs/compio/blob/master/LICENSE)
[![crates.io](https://img.shields.io/crates/v/compio-compat)](https://crates.io/crates/compio-compat)
[![docs.rs](https://img.shields.io/badge/docs.rs-compio--compat-latest)](https://docs.rs/compio-compat)
[![Check](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml)
[![Test](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml)

Run compio in other async runtimes.

## Usage

Use `compio::compat` re-exported from `compio` crate.

```rust
use compio::compat::{RuntimeCompat, TokioAdapter};

#[tokio::main]
async fn main() {
    // Create a compio runtime:
    let runtime = compio::runtime::Runtime::new().unwrap();
    // Create the compat layer:
    let runtime = RuntimeCompat::<TokioAdapter>::new(runtime).unwrap();
    // Execute your future:
    runtime.execute(async {
        // Run compio-specific code
    }).await;
}
```
