# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.11.0](https://github.com/compio-rs/compio/compare/compio-runtime-v0.10.1...compio-runtime-v0.11.0) - 2026-01-28

### Added

- *(runtime)* future combinator ([#639](https://github.com/compio-rs/compio/pull/639))
- *(runtime)* make `submit` return named future ([#615](https://github.com/compio-rs/compio/pull/615))
- *(runtime)* expose future type for submit ([#614](https://github.com/compio-rs/compio/pull/614))
- [**breaking**] fs & net feature ([#564](https://github.com/compio-rs/compio/pull/564))

### Changed

- *(runtime)* [**breaking**] submit future ([#632](https://github.com/compio-rs/compio/pull/632))
- *(driver)* extra ([#624](https://github.com/compio-rs/compio/pull/624))
- *(driver)* use `thin-cell` for `Key` ([#620](https://github.com/compio-rs/compio/pull/620))
- *(driver,runtime)* merge overlapped and flags into unified `Extra` ([#559](https://github.com/compio-rs/compio/pull/559))

### Fixed

- *(runtime)* delete op.rs ([#633](https://github.com/compio-rs/compio/pull/633))

### Other

- deploy docs ([#641](https://github.com/compio-rs/compio/pull/641))
- deny `rustdoc::broken_intra_doc_links` ([#574](https://github.com/compio-rs/compio/pull/574))
- fix broken builds ([#562](https://github.com/compio-rs/compio/pull/562))
