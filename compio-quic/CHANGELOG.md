# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.8.0](https://github.com/compio-rs/compio/compare/compio-quic-v0.7.0...compio-quic-v0.8.0) - 2026-02-27

### Added

- *(quic)* [**breaking**] sync with quinn ([#689](https://github.com/compio-rs/compio/pull/689))

### Changed

- *(quic)* `Endpoint` ([#663](https://github.com/compio-rs/compio/pull/663))

### Fixed

- *(tls)* example.com tls misconfigured ([#692](https://github.com/compio-rs/compio/pull/692))

### Other

- remove "authors" field in metadata ([#711](https://github.com/compio-rs/compio/pull/711))
- *(deps)* update rand requirement from 0.9.0 to 0.10.0 ([#671](https://github.com/compio-rs/compio/pull/671))

## [0.7.0](https://github.com/compio-rs/compio/compare/v0.17.0...v0.18.0) - 2026-01-28

### Added

- *(driver,quic)* use synchrony ([#628](https://github.com/compio-rs/compio/pull/628))
- *(quic)* sync recent changes from quinn-udp ([#592](https://github.com/compio-rs/compio/pull/592))
- [**breaking**] fs & net feature ([#564](https://github.com/compio-rs/compio/pull/564))

### Changed

- *(quic)* [**breaking**] redesign IO APIs ([#593](https://github.com/compio-rs/compio/pull/593))
- *(buf)* rename as_slice to as_init ([#594](https://github.com/compio-rs/compio/pull/594))
- set_buf_init ([#579](https://github.com/compio-rs/compio/pull/579))
- *(buf)* better IoBuf ([#555](https://github.com/compio-rs/compio/pull/555))

### Fixed

- *(net,quic)* init with uninit data for CMsgBuilder ([#583](https://github.com/compio-rs/compio/pull/583))

### Other

- deploy docs ([#641](https://github.com/compio-rs/compio/pull/641))
- *(quic)* optimization ([#588](https://github.com/compio-rs/compio/pull/588))
- deny `rustdoc::broken_intra_doc_links` ([#574](https://github.com/compio-rs/compio/pull/574))
