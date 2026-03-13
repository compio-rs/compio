# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.0](https://github.com/compio-rs/compio/compare/compio-ws-v0.3.0...compio-ws-v0.4.0) - 2026-03-13

### Added

- *(ws)* futures compat ([#713](https://github.com/compio-rs/compio/pull/713))

### Changed

- *(io,tls,ws)* [**breaking**] use pin-project-lite ([#720](https://github.com/compio-rs/compio/pull/720))

### Fixed

- unused_features ([#739](https://github.com/compio-rs/compio/pull/739))
- *(tls)* example.com tls misconfigured ([#692](https://github.com/compio-rs/compio/pull/692))
- *(driver)* statx on musl ([#669](https://github.com/compio-rs/compio/pull/669))

### Other

- remove "authors" field in metadata ([#711](https://github.com/compio-rs/compio/pull/711))

## [0.3.0](https://github.com/compio-rs/compio/compare/v0.17.0...v0.18.0) - 2026-01-28

### Added

- *(net)* [**breaking**] add `SocketOpts` support for all sockets ([#573](https://github.com/compio-rs/compio/pull/573))
- [**breaking**] fs & net feature ([#564](https://github.com/compio-rs/compio/pull/564))

### Other

- deploy docs ([#641](https://github.com/compio-rs/compio/pull/641))
- deny `rustdoc::broken_intra_doc_links` ([#574](https://github.com/compio-rs/compio/pull/574))
- fix broken builds ([#562](https://github.com/compio-rs/compio/pull/562))
