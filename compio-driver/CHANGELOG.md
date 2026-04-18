# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.12.0](https://github.com/compio-rs/compio/compare/compio-driver-v0.11.1...compio-driver-v0.12.0) - 2026-04-18

### Added

- *(driver)* add more flags to Extra ([#858](https://github.com/compio-rs/compio/pull/858))
- *(driver)* add Extra::needs_polling ([#857](https://github.com/compio-rs/compio/pull/857))
- *(driver)* recvmsg multi ([#842](https://github.com/compio-rs/compio/pull/842))
- *(driver)* buffer pool allocator ([#854](https://github.com/compio-rs/compio/pull/854))
- *(driver,iour)* allow to specify cqsize ([#834](https://github.com/compio-rs/compio/pull/834))
- *(runtime)* [**breaking**] waker-based future combinator ([#825](https://github.com/compio-rs/compio/pull/825))
- *(driver)* yield_now in push_blocking loop ([#816](https://github.com/compio-rs/compio/pull/816))
- *(driver,fs,unix)* async anonymous pipe ([#807](https://github.com/compio-rs/compio/pull/807))
- *(driver,net,unix)* async bind & listen ([#806](https://github.com/compio-rs/compio/pull/806))
- *(io)* add traits for reading/writing with ancillary data ([#717](https://github.com/compio-rs/compio/pull/717))
- implement IntoInner for PollOnce and Splice ([#792](https://github.com/compio-rs/compio/pull/792))
- *(net,win)* make `socket` & `shutdown` sync ([#789](https://github.com/compio-rs/compio/pull/789))
- *(driver)* [**breaking**] accept multi ([#747](https://github.com/compio-rs/compio/pull/747))
- *(driver)* fallback for zerocopy ops ([#755](https://github.com/compio-rs/compio/pull/755))
- *(driver)* send zerocopy for Linux ([#754](https://github.com/compio-rs/compio/pull/754))
- update thin-cell ([#738](https://github.com/compio-rs/compio/pull/738))
- *(driver)* multishot op ([#715](https://github.com/compio-rs/compio/pull/715))
- *(driver)* make stub & iocp driver non-send and test ([#728](https://github.com/compio-rs/compio/pull/728))
- *(driver)* add register_files/unregister_files for io_uring fixed-file ops ([#718](https://github.com/compio-rs/compio/pull/718))
- *(driver)* entry fallback ([#716](https://github.com/compio-rs/compio/pull/716))
- *(driver)* add recv_from_managed operation support ([#709](https://github.com/compio-rs/compio/pull/709))
- *(fs)* dirfd support ([#703](https://github.com/compio-rs/compio/pull/703))
- *(driver,unix)* [**breaking**] support dirfd relative fs ops ([#699](https://github.com/compio-rs/compio/pull/699))
- *(driver,iocp)* impl AsFd for borrowed handle ([#694](https://github.com/compio-rs/compio/pull/694))
- *(driver)* force OpCode support ([#690](https://github.com/compio-rs/compio/pull/690))
- *(dispatcher)* block standard POSIX signals on worker threads ([#672](https://github.com/compio-rs/compio/pull/672))
- *(runtime)* cancel & future combinator ([#665](https://github.com/compio-rs/compio/pull/665))
- *(driver)* cancel token ([#660](https://github.com/compio-rs/compio/pull/660))

### Changed

- [**breaking**] use rustix ([#876](https://github.com/compio-rs/compio/pull/876))
- *(driver)* adjust sys layout ([#870](https://github.com/compio-rs/compio/pull/870))
- *(driver)* use WeakKey for Cancel ([#864](https://github.com/compio-rs/compio/pull/864))
- *(driver)* handle panicking ([#853](https://github.com/compio-rs/compio/pull/853))
- *(driver)* require Default for Control ([#859](https://github.com/compio-rs/compio/pull/859))
- [**breaking**] buffer pool & managed IO ([#820](https://github.com/compio-rs/compio/pull/820))
- *(driver,unix)* use control api ([#804](https://github.com/compio-rs/compio/pull/804))
- *(driver,iocp)* use control api ([#803](https://github.com/compio-rs/compio/pull/803))
- *(driver)* get rid of pin ([#758](https://github.com/compio-rs/compio/pull/758))
- *(net)* adjust `send*` methods ([#770](https://github.com/compio-rs/compio/pull/770))
- *(driver)* record multishot results in ops ([#748](https://github.com/compio-rs/compio/pull/748))
- *(driver)* [**breaking**] make update_waker take &Key ([#742](https://github.com/compio-rs/compio/pull/742))
- *(driver)* shared fd ([#661](https://github.com/compio-rs/compio/pull/661))

### Fixed

- *(driver,unix)* add some control API usages ([#832](https://github.com/compio-rs/compio/pull/832))
- *(driver)* avoid leak if not consumed ([#809](https://github.com/compio-rs/compio/pull/809))
- *(driver,fs)* add Sync on fds of AsyncifyFd* ([#805](https://github.com/compio-rs/compio/pull/805))
- *(driver)* memory leaks on drop ([#769](https://github.com/compio-rs/compio/pull/769))
- *(driver)* rust-analyzer is confused by Extra ([#740](https://github.com/compio-rs/compio/pull/740))
- unused_features ([#739](https://github.com/compio-rs/compio/pull/739))
- *(driver,iour)* make `Driver` non-`Send` ([#727](https://github.com/compio-rs/compio/pull/727))
- *(driver,net)* [**breaking**] to/from/msg have optional address ([#721](https://github.com/compio-rs/compio/pull/721))
- *(driver,stub)* allow creation ([#705](https://github.com/compio-rs/compio/pull/705))
- *(driver,unix)* `set_result` for `OpenFile` & `CreateSocket` ([#701](https://github.com/compio-rs/compio/pull/701))
- *(driver)* key is not unique when spawn_blocking ([#675](https://github.com/compio-rs/compio/pull/675))
- *(driver)* statx on musl ([#669](https://github.com/compio-rs/compio/pull/669))
- *(driver)* the fusion driver with polling variant ([#670](https://github.com/compio-rs/compio/pull/670))

### Other

- *(driver)* release buffer pool ([#810](https://github.com/compio-rs/compio/pull/810))
- *(driver)* read multi on pipe ([#760](https://github.com/compio-rs/compio/pull/760))
- remove "authors" field in metadata ([#711](https://github.com/compio-rs/compio/pull/711))
- *(driver)* fix doc for Dispatchable ([#693](https://github.com/compio-rs/compio/pull/693))

## [0.11.1](https://github.com/compio-rs/compio/compare/v0.17.0...v0.18.0) - 2026-01-28

### Added

- *(driver)* [**breaking**] full iour fallback ([#656](https://github.com/compio-rs/compio/pull/656))
- *(runtime)* future combinator ([#639](https://github.com/compio-rs/compio/pull/639))
- *(fs)* splice ([#635](https://github.com/compio-rs/compio/pull/635))
- *(driver)* fake splice test ([#638](https://github.com/compio-rs/compio/pull/638))
- *(driver,iour)* support personality ([#630](https://github.com/compio-rs/compio/pull/630))
- *(driver,quic)* use synchrony ([#628](https://github.com/compio-rs/compio/pull/628))
- *(driver)* use nix instead of rustix ([#627](https://github.com/compio-rs/compio/pull/627))
- *(driver,poll)* multi fd ([#623](https://github.com/compio-rs/compio/pull/623))
- *(driver)* ensure that all supported statx fields are filled ([#625](https://github.com/compio-rs/compio/pull/625))
- *(driver)* add splice ([#609](https://github.com/compio-rs/compio/pull/609))
- *(driver, fs)* truncate file ([#611](https://github.com/compio-rs/compio/pull/611))
- *(driver,unix)* use stat64 if possible ([#597](https://github.com/compio-rs/compio/pull/597))
- [**breaking**] fs & net feature ([#564](https://github.com/compio-rs/compio/pull/564))
- *(driver)* distinguish Read/Write & Recv/Send ([#567](https://github.com/compio-rs/compio/pull/567))
- *(driver)* stablize aliases of OpCode ([#566](https://github.com/compio-rs/compio/pull/566))

### Changed

- *(driver)* [**breaking**] make opcode unsafe ([#650](https://github.com/compio-rs/compio/pull/650))
- *(runtime)* [**breaking**] submit future ([#632](https://github.com/compio-rs/compio/pull/632))
- *(driver)* extra ([#624](https://github.com/compio-rs/compio/pull/624))
- *(driver,poll)* with_events ([#622](https://github.com/compio-rs/compio/pull/622))
- *(driver)* use `thin-cell` for `Key` ([#620](https://github.com/compio-rs/compio/pull/620))
- *(driver)* limit usage of Key::new_unchecked ([#618](https://github.com/compio-rs/compio/pull/618))
- *(driver)* make Key less unsafe ([#616](https://github.com/compio-rs/compio/pull/616))
- *(buf)* rename as_slice to as_init ([#594](https://github.com/compio-rs/compio/pull/594))
- set_buf_init ([#579](https://github.com/compio-rs/compio/pull/579))
- *(driver,iocp)* [**breaking**] make `OpCode::cancel` safe ([#575](https://github.com/compio-rs/compio/pull/575))
- *(driver)* use pin_project_lite for Ops ([#570](https://github.com/compio-rs/compio/pull/570))
- *(driver,runtime)* merge overlapped and flags into unified `Extra` ([#559](https://github.com/compio-rs/compio/pull/559))
- *(buf)* better IoBuf ([#555](https://github.com/compio-rs/compio/pull/555))

### Fixed

- *(driver,iour)* cancel leaks the key ([#652](https://github.com/compio-rs/compio/pull/652))
- *(driver)* user_data does not exist ([#643](https://github.com/compio-rs/compio/pull/643))
- *(driver)* multi fd ([#636](https://github.com/compio-rs/compio/pull/636))
- *(driver, IOCP)* cap buf, sys_slice length to u32 ([#613](https://github.com/compio-rs/compio/pull/613))
- *(driver,stub)* compile ([#612](https://github.com/compio-rs/compio/pull/612))
- *(iour)* cap buf len to u32::MAX ([#600](https://github.com/compio-rs/compio/pull/600)) ([#601](https://github.com/compio-rs/compio/pull/601))
- *(buf,driver)* safety around `set_len` ([#585](https://github.com/compio-rs/compio/pull/585))
- *(driver,iocp)* send* & recv* might receive dangling ptrs ([#572](https://github.com/compio-rs/compio/pull/572))
- *(driver)* stub driver is broken ([#560](https://github.com/compio-rs/compio/pull/560))

### Other

- deploy docs ([#641](https://github.com/compio-rs/compio/pull/641))
- *(driver)* update `SockAddrStorage` usage ([#599](https://github.com/compio-rs/compio/pull/599))
- deny `rustdoc::broken_intra_doc_links` ([#574](https://github.com/compio-rs/compio/pull/574))
- fix broken builds ([#562](https://github.com/compio-rs/compio/pull/562))
