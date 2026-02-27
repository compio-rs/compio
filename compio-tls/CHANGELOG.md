# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.10.0](https://github.com/compio-rs/compio/compare/compio-tls-v0.9.0...compio-tls-v0.10.0) - 2026-02-27

### Added

- *(tls)* support py-dynamic-openssl ([#700](https://github.com/compio-rs/compio/pull/700))
- *(tls)* add LazyConfigAcceptor for rustls ([#686](https://github.com/compio-rs/compio/pull/686))

### Fixed

- *(tls,io)* multiple native-tls issues ([#698](https://github.com/compio-rs/compio/pull/698))
- *(tls)* example.com tls misconfigured ([#692](https://github.com/compio-rs/compio/pull/692))
- *(driver)* statx on musl ([#669](https://github.com/compio-rs/compio/pull/669))

### Other

- remove "authors" field in metadata ([#711](https://github.com/compio-rs/compio/pull/711))

## [0.9.0](https://github.com/compio-rs/compio/compare/v0.17.0...v0.18.0) - 2026-01-28

### Added

- [**breaking**] fs & net feature ([#564](https://github.com/compio-rs/compio/pull/564))

### Changed

- *(buf)* rename as_slice to as_init ([#594](https://github.com/compio-rs/compio/pull/594))
- set_buf_init ([#579](https://github.com/compio-rs/compio/pull/579))
- *(buf)* better IoBuf ([#555](https://github.com/compio-rs/compio/pull/555))

### Other

- deploy docs ([#641](https://github.com/compio-rs/compio/pull/641))
- deny `rustdoc::broken_intra_doc_links` ([#574](https://github.com/compio-rs/compio/pull/574))
- fix intra doc links ([#554](https://github.com/compio-rs/compio/pull/554))
