# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.12.0](https://github.com/compio-rs/compio/compare/compio-net-v0.11.0...compio-net-v0.12.0) - 2026-02-27

### Added

- *(net)* add recv_from_managed ([#710](https://github.com/compio-rs/compio/pull/710))
- *(runtime)* [**breaking**] remove event ([#707](https://github.com/compio-rs/compio/pull/707))

### Changed

- [**breaking**] move {Async,Poll}Fd to runtime ([#662](https://github.com/compio-rs/compio/pull/662))

### Fixed

- *(driver,unix)* `set_result` for `OpenFile` & `CreateSocket` ([#701](https://github.com/compio-rs/compio/pull/701))

### Other

- remove "authors" field in metadata ([#711](https://github.com/compio-rs/compio/pull/711))
- compio::runtime instead of compio_runtime ([#664](https://github.com/compio-rs/compio/pull/664))

## [0.11.0](https://github.com/compio-rs/compio/compare/v0.17.0...v0.18.0) - 2026-01-28

### Added

- *(net)* [**breaking**] add `SocketOpts` support for all sockets ([#573](https://github.com/compio-rs/compio/pull/573))
- [**breaking**] fs & net feature ([#564](https://github.com/compio-rs/compio/pull/564))
- *(driver)* distinguish Read/Write & Recv/Send ([#567](https://github.com/compio-rs/compio/pull/567))

### Changed

- *(runtime)* [**breaking**] submit future ([#632](https://github.com/compio-rs/compio/pull/632))
- set_buf_init ([#579](https://github.com/compio-rs/compio/pull/579))
- *(driver,runtime)* merge overlapped and flags into unified `Extra` ([#559](https://github.com/compio-rs/compio/pull/559))

### Fixed

- *(buf,driver)* safety around `set_len` ([#585](https://github.com/compio-rs/compio/pull/585))
- *(net,quic)* init with uninit data for CMsgBuilder ([#583](https://github.com/compio-rs/compio/pull/583))

### Other

- deploy docs ([#641](https://github.com/compio-rs/compio/pull/641))
- deny `rustdoc::broken_intra_doc_links` ([#574](https://github.com/compio-rs/compio/pull/574))
