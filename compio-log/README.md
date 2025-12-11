<div align="center">
    <a href='https://compio.rs'>
        <img height="150" src="https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-with-text.svg">
    </a>
</div>

---

# compio-log

[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/compio-rs/compio/blob/master/LICENSE)
[![crates.io](https://img.shields.io/crates/v/compio-log)](https://crates.io/crates/compio-log)
[![docs.rs](https://img.shields.io/badge/docs.rs-compio--log-latest)](https://docs.rs/compio-log)
[![Check](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml)
[![Test](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml)

Logging utilities for compio.

This crate provides internal logging support for compio, built on top of the `tracing` crate. It's used throughout the compio ecosystem for instrumentation and debugging. You shouldn't need to use crate.

To see logs from compio, enable `enable_log` feature of `compio`, and set up a `tracing` subscriber in your application or testing environment.
