<div align="center">
    <a href='https://compio.rs'>
        <img height="150" src="https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-with-text.svg">
    </a>
</div>

---

# compio-driver

[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/compio-rs/compio/blob/master/LICENSE)
[![crates.io](https://img.shields.io/crates/v/compio-driver)](https://crates.io/crates/compio-driver)
[![docs.rs](https://img.shields.io/badge/docs.rs-compio--driver-latest)](https://docs.rs/compio-driver)
[![Check](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml)
[![Test](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml)

Low-level driver for compio.

This crate provides the platform-specific driver (`Proactor`) of compio. It abstracts over different OS backends:

- **Windows**: IOCP (IO Completion Ports)
- **Linux**: io_uring (with optional polling fallback)
- **Other Unix platforms**: polling

The driver manages the submission and completion of IO operations, providing a unified interface regardless of the underlying platform mechanism.

## Usage

This crate is typically used indirectly through compio runtime, but can also be used directly for low-level control over the IO driver. See examples in compio crate.
