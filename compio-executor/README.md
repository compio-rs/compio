<div align="center">
    <a href='https://compio.rs'>
        <img height="150" src="https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-with-text.svg">
    </a>
</div>

---

# compio-executor

[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/compio-rs/compio/blob/master/LICENSE)
[![crates.io](https://img.shields.io/crates/v/compio-executor)](https://crates.io/crates/compio-executor)
[![docs.rs](https://img.shields.io/badge/docs.rs-compio--runtime-latest)](https://docs.rs/compio-executor)
[![Check](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_check.yml)
[![Test](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml/badge.svg)](https://github.com/compio-rs/compio/actions/workflows/ci_test.yml)

Low-level executor for compio.

This crate provides the async executor that coordinates task execution without any IO. It assumes thread-per-core (singlethreaded) with compromises to support multithreaded wakers.

## Usage

You should use `compio-runtime` or `compio::runtime` instead of using this crate directly if you want I/O support (io-uring, IOCP etc) from compio.
