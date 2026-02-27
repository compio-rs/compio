# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.8.1](https://github.com/compio-rs/compio/compare/compio-buf-v0.8.0...compio-buf-v0.8.1) - 2026-02-27

### Added

- *(buf)* add into_parts for BufResult ([#712](https://github.com/compio-rs/compio/pull/712))
- *(buf)* add support for memmap2 ([#684](https://github.com/compio-rs/compio/pull/684))

### Other

- remove "authors" field in metadata ([#711](https://github.com/compio-rs/compio/pull/711))
- release 0.18 ([#653](https://github.com/compio-rs/compio/pull/653))

## [0.8.0](https://github.com/compio-rs/compio/compare/v0.17.0...v0.18.0) - 2026-01-28

### Added

- *(io)* [**breaking**] support generic buffer for `Framed` ([#642](https://github.com/compio-rs/compio/pull/642))
- *(driver,poll)* multi fd ([#623](https://github.com/compio-rs/compio/pull/623))
- *(buf)* add `reserve{,exact}` to `IoBufMut` ([#578](https://github.com/compio-rs/compio/pull/578))
- [**breaking**] fs & net feature ([#564](https://github.com/compio-rs/compio/pull/564))
- *(buf)* make BufResult compatible with more Result types ([#569](https://github.com/compio-rs/compio/pull/569))

### Changed

- *(buf)* rename as_slice to as_init ([#594](https://github.com/compio-rs/compio/pull/594))
- set_buf_init ([#579](https://github.com/compio-rs/compio/pull/579))
- *(buf)* better IoBuf ([#555](https://github.com/compio-rs/compio/pull/555))

### Fixed

- *(buf,driver)* safety around `set_len` ([#585](https://github.com/compio-rs/compio/pull/585))
- *(buf)* `BorrowedCursor::advance` is unsafe ([#558](https://github.com/compio-rs/compio/pull/558))

### Other

- deploy docs ([#641](https://github.com/compio-rs/compio/pull/641))
- deny `rustdoc::broken_intra_doc_links` ([#574](https://github.com/compio-rs/compio/pull/574))
- fix broken builds ([#562](https://github.com/compio-rs/compio/pull/562))
