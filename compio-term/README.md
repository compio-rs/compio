<div align="center">
    <a href='https://compio.rs'>
        <img height="150" src="https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-with-text.svg">
    </a>
</div>

---

# compio-term

[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/compio-rs/compio/blob/master/LICENSE)
[![crates.io](https://img.shields.io/crates/v/compio-term)](https://crates.io/crates/compio-term)
[![docs.rs](https://img.shields.io/badge/docs.rs-compio--term-latest)](https://docs.rs/compio-term)
[![Check](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml)
[![Test](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml)

`compio-term` provides completion-based terminal input and output for Compio based on [crossterm](https://github.com/crossterm-rs/crossterm).

The crate provides an async event stream based on compio and an asynchronous command queue to write commands to any `AsyncWrite`. Crossterm's `cursor`, `style`, `terminal`, and `tty` modules are re-exported from the crate root.

## Backends

| Platform                  | Event mechanism                                                    |
| ------------------------- | ------------------------------------------------------------------ |
| Linux and Android         | Managed multishot reads, including native io_uring multishot reads |
| Apple                     | Managed reads for terminal stdin and timer polling for `/dev/tty`  |
| BSD, illumos, and Solaris | Managed reads over the platform polling backend                    |
| Other Unix                | Timer polling with nonblocking rustix reads                        |
| Windows                   | Compio event waits and `ReadConsoleInputW`                          |

Mouse, focus, bracketed-paste, and enhanced keyboard events require the related Crossterm command to be enabled by the caller.
