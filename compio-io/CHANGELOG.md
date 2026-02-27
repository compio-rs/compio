# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.9.1](https://github.com/compio-rs/compio/compare/compio-io-v0.9.0...compio-io-v0.9.1) - 2026-02-27

### Added

- *(io)* add duplex forwarding for BufReader/BufWriter ([#695](https://github.com/compio-rs/compio/pull/695))

### Fixed

- *(tls,io)* multiple native-tls issues ([#698](https://github.com/compio-rs/compio/pull/698))
- *(io)* flush manually in poll_close ([#681](https://github.com/compio-rs/compio/pull/681))
- *(driver)* the fusion driver with polling variant ([#670](https://github.com/compio-rs/compio/pull/670))

### Other

- remove "authors" field in metadata ([#711](https://github.com/compio-rs/compio/pull/711))

## [0.9.0](https://github.com/compio-rs/compio/compare/v0.17.0...v0.18.0) - 2026-01-28

### Added

- *(io)* [**breaking**] support generic buffer for `Framed` ([#642](https://github.com/compio-rs/compio/pull/642))
- *(buf)* add `reserve{,exact}` to `IoBufMut` ([#578](https://github.com/compio-rs/compio/pull/578))
- [**breaking**] fs & net feature ([#564](https://github.com/compio-rs/compio/pull/564))

### Changed

- *(io)* [**breaking**] use synchrony for split ([#640](https://github.com/compio-rs/compio/pull/640))
- *(io)* [**breaking**] enable fallible frame extraction ([#631](https://github.com/compio-rs/compio/pull/631))
- *(buf)* rename as_slice to as_init ([#594](https://github.com/compio-rs/compio/pull/594))
- set_buf_init ([#579](https://github.com/compio-rs/compio/pull/579))
- *(buf)* better IoBuf ([#555](https://github.com/compio-rs/compio/pull/555))

### Fixed

- *(io)* document mismatch from behavior ([#557](https://github.com/compio-rs/compio/pull/557))

### Other

- deploy docs ([#641](https://github.com/compio-rs/compio/pull/641))
- deny `rustdoc::broken_intra_doc_links` ([#574](https://github.com/compio-rs/compio/pull/574))
- fix intra doc links ([#554](https://github.com/compio-rs/compio/pull/554))
