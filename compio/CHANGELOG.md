# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.18.0](https://github.com/compio-rs/compio/compare/v0.17.0...v0.18.0) - 2026-01-28

### Added

- *(runtime)* future combinator ([#639](https://github.com/compio-rs/compio/pull/639))
- *(driver,quic)* use synchrony ([#628](https://github.com/compio-rs/compio/pull/628))
- [**breaking**] fs & net feature ([#564](https://github.com/compio-rs/compio/pull/564))

### Changed

- *(io)* [**breaking**] use synchrony for split ([#640](https://github.com/compio-rs/compio/pull/640))
- *(buf)* rename as_slice to as_init ([#594](https://github.com/compio-rs/compio/pull/594))
- set_buf_init ([#579](https://github.com/compio-rs/compio/pull/579))
- *(driver,runtime)* merge overlapped and flags into unified `Extra` ([#559](https://github.com/compio-rs/compio/pull/559))
- *(buf)* better IoBuf ([#555](https://github.com/compio-rs/compio/pull/555))

### Fixed

- *(bench)* replace `read-all` throughput with TOTAL_SIZE ([#561](https://github.com/compio-rs/compio/pull/561))

### Other

- deploy docs ([#641](https://github.com/compio-rs/compio/pull/641))
- deny `rustdoc::broken_intra_doc_links` ([#574](https://github.com/compio-rs/compio/pull/574))
- fix broken builds ([#562](https://github.com/compio-rs/compio/pull/562))
