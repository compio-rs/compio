# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.8.0-rc.2](https://github.com/compio-rs/compio/compare/compio-quic-v0.8.0-rc.1...compio-quic-v0.8.0-rc.2) - 2026-04-26

### Other

- add more targets for docs.rs ([#896](https://github.com/compio-rs/compio/pull/896))

## 0.8.0-rc.1 - 2026-04-20

### Added

- [**breaking**] compio-executor ([#790](https://github.com/compio-rs/compio/pull/790))
- *(io)* fix ancillary API to avoid UB ([#737](https://github.com/compio-rs/compio/pull/737))
- export more error types ([#782](https://github.com/compio-rs/compio/pull/782))
- add support try_recv_datagram ([#744](https://github.com/compio-rs/compio/pull/744))
- *(quic)* [**breaking**] sync with quinn ([#689](https://github.com/compio-rs/compio/pull/689))

### Changed

- *(io,quic)* move quic Ancillary to io ([#734](https://github.com/compio-rs/compio/pull/734))
- *(io,net)* move cmsg to io ancillary ([#730](https://github.com/compio-rs/compio/pull/730))
- *(quic)* `Endpoint` ([#663](https://github.com/compio-rs/compio/pull/663))

### Fixed

- *(quic)* drop connection explicitly ([#827](https://github.com/compio-rs/compio/pull/827))
- *(quic)* test requires synchrony/async-flag ([#823](https://github.com/compio-rs/compio/pull/823))
- unused_features ([#739](https://github.com/compio-rs/compio/pull/739))
- *(tls)* example.com tls misconfigured ([#692](https://github.com/compio-rs/compio/pull/692))

### Other

- *(tls)* a local self-signed server ([#829](https://github.com/compio-rs/compio/pull/829))
- *(quic)* avoid shutting down early ([#818](https://github.com/compio-rs/compio/pull/818))
- *(quic)* shutdown endpoints ([#813](https://github.com/compio-rs/compio/pull/813))
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
