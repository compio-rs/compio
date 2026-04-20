# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.9.0](https://github.com/compio-rs/compio/compare/v0.18.0...v0.19.0) - 2026-04-20

### Added

- [**breaking**] compio-executor ([#790](https://github.com/compio-rs/compio/pull/790))

### Changed

- *(driver)* require Default for Control ([#859](https://github.com/compio-rs/compio/pull/859))
- [**breaking**] rename all "canceled" to "cancelled" ([#826](https://github.com/compio-rs/compio/pull/826))
- [**breaking**] buffer pool & managed IO ([#820](https://github.com/compio-rs/compio/pull/820))
- *(driver)* get rid of pin ([#758](https://github.com/compio-rs/compio/pull/758))

### Fixed

- unused_features ([#739](https://github.com/compio-rs/compio/pull/739))

### Other

- remove "authors" field in metadata ([#711](https://github.com/compio-rs/compio/pull/711))

## [0.8.0](https://github.com/compio-rs/compio/compare/v0.17.0...v0.18.0) - 2026-01-28

### Added

- [**breaking**] fs & net feature ([#564](https://github.com/compio-rs/compio/pull/564))
- *(driver)* distinguish Read/Write & Recv/Send ([#567](https://github.com/compio-rs/compio/pull/567))

### Changed

- *(runtime)* [**breaking**] submit future ([#632](https://github.com/compio-rs/compio/pull/632))
- *(driver,runtime)* merge overlapped and flags into unified `Extra` ([#559](https://github.com/compio-rs/compio/pull/559))

### Fixed

- *(buf,driver)* safety around `set_len` ([#585](https://github.com/compio-rs/compio/pull/585))

### Other

- deploy docs ([#641](https://github.com/compio-rs/compio/pull/641))
- deny `rustdoc::broken_intra_doc_links` ([#574](https://github.com/compio-rs/compio/pull/574))
