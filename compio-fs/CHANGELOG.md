# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## 0.12.0-rc.1 - 2026-04-20

### Added

- *(runtime,fs,net)* high-level multishot ([#830](https://github.com/compio-rs/compio/pull/830))
- *(driver,fs,unix)* async anonymous pipe ([#807](https://github.com/compio-rs/compio/pull/807))
- *(io)* copy-bidirectional ([#800](https://github.com/compio-rs/compio/pull/800))
- [**breaking**] compio-executor ([#790](https://github.com/compio-rs/compio/pull/790))
- *(io)* allow create a stream out of AsyncRead for BytesFramed ([#767](https://github.com/compio-rs/compio/pull/767))
- *(tls,fs)* fix tests ([#757](https://github.com/compio-rs/compio/pull/757))
- *(fs)* dirfd support ([#703](https://github.com/compio-rs/compio/pull/703))
- *(driver,unix)* [**breaking**] support dirfd relative fs ops ([#699](https://github.com/compio-rs/compio/pull/699))

### Changed

- [**breaking**] use rustix ([#876](https://github.com/compio-rs/compio/pull/876))
- *(driver)* require Default for Control ([#859](https://github.com/compio-rs/compio/pull/859))
- [**breaking**] rename all "canceled" to "cancelled" ([#826](https://github.com/compio-rs/compio/pull/826))
- [**breaking**] buffer pool & managed IO ([#820](https://github.com/compio-rs/compio/pull/820))
- *(driver)* get rid of pin ([#758](https://github.com/compio-rs/compio/pull/758))
- [**breaking**] move {Async,Poll}Fd to runtime ([#662](https://github.com/compio-rs/compio/pull/662))

### Fixed

- *(driver,fs)* add Sync on fds of AsyncifyFd* ([#805](https://github.com/compio-rs/compio/pull/805))
- unused_features ([#739](https://github.com/compio-rs/compio/pull/739))
- *(driver,unix)* `set_result` for `OpenFile` & `CreateSocket` ([#701](https://github.com/compio-rs/compio/pull/701))

### Other

- *(fs,net)* comments on `close` ([#821](https://github.com/compio-rs/compio/pull/821))
- remove "authors" field in metadata ([#711](https://github.com/compio-rs/compio/pull/711))
- compio::runtime instead of compio_runtime ([#664](https://github.com/compio-rs/compio/pull/664))

## [0.11.0](https://github.com/compio-rs/compio/compare/v0.17.0...v0.18.0) - 2026-01-28

### Added

- *(runtime)* future combinator ([#639](https://github.com/compio-rs/compio/pull/639))
- *(fs)* splice ([#635](https://github.com/compio-rs/compio/pull/635))
- *(driver, fs)* truncate file ([#611](https://github.com/compio-rs/compio/pull/611))
- *(driver,unix)* use stat64 if possible ([#597](https://github.com/compio-rs/compio/pull/597))
- [**breaking**] fs & net feature ([#564](https://github.com/compio-rs/compio/pull/564))
- *(driver)* distinguish Read/Write & Recv/Send ([#567](https://github.com/compio-rs/compio/pull/567))

### Changed

- *(runtime)* [**breaking**] submit future ([#632](https://github.com/compio-rs/compio/pull/632))
- *(buf)* rename as_slice to as_init ([#594](https://github.com/compio-rs/compio/pull/594))
- set_buf_init ([#579](https://github.com/compio-rs/compio/pull/579))
- *(driver,iocp)* [**breaking**] make `OpCode::cancel` safe ([#575](https://github.com/compio-rs/compio/pull/575))
- *(driver,runtime)* merge overlapped and flags into unified `Extra` ([#559](https://github.com/compio-rs/compio/pull/559))
- *(buf)* better IoBuf ([#555](https://github.com/compio-rs/compio/pull/555))

### Fixed

- *(buf,driver)* safety around `set_len` ([#585](https://github.com/compio-rs/compio/pull/585))

### Other

- deploy docs ([#641](https://github.com/compio-rs/compio/pull/641))
- deny `rustdoc::broken_intra_doc_links` ([#574](https://github.com/compio-rs/compio/pull/574))
- fix intra doc links ([#554](https://github.com/compio-rs/compio/pull/554))
