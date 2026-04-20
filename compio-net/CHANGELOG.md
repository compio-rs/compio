# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.12.0](https://github.com/compio-rs/compio/compare/v0.18.0...v0.19.0) - 2026-04-20

### Added

- *(net, driver, buf)* support socket state ([#861](https://github.com/compio-rs/compio/pull/861))
- *(net)* multi & managed for recvfrom & recvmsg ([#838](https://github.com/compio-rs/compio/pull/838))
- *(runtime,fs,net)* high-level multishot ([#830](https://github.com/compio-rs/compio/pull/830))
- *(runtime)* [**breaking**] waker-based future combinator ([#825](https://github.com/compio-rs/compio/pull/825))
- *(driver,net,unix)* async bind & listen ([#806](https://github.com/compio-rs/compio/pull/806))
- *(io)* copy-bidirectional ([#800](https://github.com/compio-rs/compio/pull/800))
- *(io)* add traits for reading/writing with ancillary data ([#717](https://github.com/compio-rs/compio/pull/717))
- [**breaking**] compio-executor ([#790](https://github.com/compio-rs/compio/pull/790))
- *(io)* fix ancillary API to avoid UB ([#737](https://github.com/compio-rs/compio/pull/737))
- *(net,win)* make `socket` & `shutdown` sync ([#789](https://github.com/compio-rs/compio/pull/789))
- *(net)* set backlog in SocketOpts ([#781](https://github.com/compio-rs/compio/pull/781))
- *(net)* incoming stream ([#759](https://github.com/compio-rs/compio/pull/759))
- *(net)* zerocopy API ([#756](https://github.com/compio-rs/compio/pull/756))
- *(driver)* [**breaking**] accept multi ([#747](https://github.com/compio-rs/compio/pull/747))
- *(net)* add recv_from_managed ([#710](https://github.com/compio-rs/compio/pull/710))
- *(runtime)* [**breaking**] remove event ([#707](https://github.com/compio-rs/compio/pull/707))

### Changed

- [**breaking**] use rustix ([#876](https://github.com/compio-rs/compio/pull/876))
- *(io)* accept multiple buffer types in AncillaryBuilder ([#795](https://github.com/compio-rs/compio/pull/795))
- *(net)* [**breaking**] add TcpSocket & UnixSocket ([#817](https://github.com/compio-rs/compio/pull/817))
- [**breaking**] rename all "canceled" to "cancelled" ([#826](https://github.com/compio-rs/compio/pull/826))
- [**breaking**] buffer pool & managed IO ([#820](https://github.com/compio-rs/compio/pull/820))
- *(net)* adjust `send*` methods ([#770](https://github.com/compio-rs/compio/pull/770))
- *(io,net)* move cmsg to io ancillary ([#730](https://github.com/compio-rs/compio/pull/730))
- [**breaking**] move {Async,Poll}Fd to runtime ([#662](https://github.com/compio-rs/compio/pull/662))

### Fixed

- *(net)* no spawn_blocking in Incoming ([#872](https://github.com/compio-rs/compio/pull/872))
- *(net)* flag MSG_NOSIGNAL for send_msg ([#835](https://github.com/compio-rs/compio/pull/835))
- *(net)* handle shutdown errors ([#808](https://github.com/compio-rs/compio/pull/808))
- *(net)* uds buffer pool test ([#811](https://github.com/compio-rs/compio/pull/811))
- *(net)* unix socket tests on Windows ([#768](https://github.com/compio-rs/compio/pull/768))
- unused_features ([#739](https://github.com/compio-rs/compio/pull/739))
- *(driver,net)* [**breaking**] to/from/msg have optional address ([#721](https://github.com/compio-rs/compio/pull/721))
- *(driver,unix)* `set_result` for `OpenFile` & `CreateSocket` ([#701](https://github.com/compio-rs/compio/pull/701))

### Other

- *(net)* multishot on io-uring ([#860](https://github.com/compio-rs/compio/pull/860))
- ignore instead of cfg out ([#845](https://github.com/compio-rs/compio/pull/845))
- update ([#844](https://github.com/compio-rs/compio/pull/844))
- *(fs,net)* comments on `close` ([#821](https://github.com/compio-rs/compio/pull/821))
- address sanitizer for Linux ([#814](https://github.com/compio-rs/compio/pull/814))
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
