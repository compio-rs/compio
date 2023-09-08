//! Registered file descriptor interface with default platform independent
//! implementation

/// Registered file descriptor is valid only for owning thread
///
///
/// It's not `Send` and not `Sync`
use std::{
    io,
    marker::PhantomData,
    mem::{transmute, MaybeUninit},
    os::fd::{IntoRawFd, OwnedFd, RawFd},
};

use super::Poller;
use crate::buf::Slice;

/// Registered fd is owned by a single thread
///
/// Concurrent operations could copy it.
/// If previously registered fd has become unregistered subsequent registered fd
/// usage could result in IO error
#[derive(Debug, Clone, Copy)]
pub struct RegisteredFd {
    // 31 bit fd idx
    fd_idx: u32,
    _nor_send_nor_sync: PhantomData<*const ()>,
}

impl RegisteredFd {
    // not allocated slot for registered file descriptors
    // we use this value instead of -1 because we work with unsigned offset values
    // the value is outside of the i32 range used by Linux kernel
    const NOT_ALLOCATED: Self = Self::new(u32::MAX);

    // private
    const fn new(fd_idx: RawFd) -> Self {
        Self {
            fd_idx,
            _nor_send_nor_sync: PhantomData,
        }
    }
}

#[repr(transparent)]
pub struct RegisteredFdUpdate(RawFd);

impl RegisteredFdUpdate {
    const SKIP_UPDATE: RegisteredFdUpdate = -2;
    const UNREGISTER: RegisteredFdUpdate = -1;

    pub const fn unregister() -> Self {
        Self::UNREGISTER
    }

    pub const fn skip() -> Self {
        Self::SKIP_UPDATE
    }

    pub const fn replace(replacement: RegisteredFd) -> Self {
        Self(replacement.into_raw_fd())
    }
}

/// Public registration API
pub trait RegisteredFileDescriptors: Poller {
    /// Allocate boxed array of raw fds, initialized with -1 (not allocated)
    /// value
    ///
    /// Use cases:
    /// * Runtime registers empty descriptor storage
    /// * Runtime could preallocate descriptor storage and let user to
    ///   initialize it
    /// incrementally
    /// * drivers for platforms that don't support registered
    /// files could emulate it on top of this storage
    fn allocate_file_descriptors<const N: usize>(&mut self) -> Box<[RawFd; N]> {
        // Create an uninitialized array of `MaybeUninit`. The `assume_init` is
        // safe because the type we are claiming to have initialized here is a
        // bunch of `MaybeUninit`s, which do not require initialization.
        let mut data: Box<[MaybeUninit<RawFd>; N]> = unsafe { MaybeUninit::uninit().assume_init() };

        // Dropping a `MaybeUninit` does nothing, so if there is a panic during this
        // loop, we have a memory leak, but there is no memory safety issue.
        for elem in &mut data[..] {
            elem.write(RegisteredFd::NOT_ALLOCATED);
        }

        // SAFETY: Everything is initialized. Transmute the array to the
        // initialized type.
        unsafe { transmute::<_, Box<[RawFd; N]>>(data) }
    }

    /// Register owned files synchronously
    ///
    /// Userspace descriptors will be closed after registration unless driver
    /// emulates registration
    fn register_files<const N: usize>(fds: [OwnedFd; N]) -> io::Result<()> {}
    /// Register owned files synchronously. Boxed slice variant
    ///
    /// Userspace descriptors will be closed after registration unless driver
    /// emulates registration
    fn register_boxed_files(fds: Box<[OwnedFd]>) -> io::Result<()> {}
    /// Register files synchronously
    ///
    /// On Linux will block until all inflight operations will finish
    fn register_raw_files(fds: &[RawFd]) -> io::Result<()> {}
    /// Replace registered files synchronously
    ///
    /// On Linux will block until all inflight operations will finish
    fn register_files_update(offset: u32, fds: &[RegisteredFdUpdate]) {}
    /// Replace registered files asynchronously
    ///
    /// The owned fds slice with the underlying allocation will be returned by a
    /// runtime method when operation is completed
    fn push_register_files_update(offset: u32, fds: Slice<Box<[RegisteredFdUpdate]>>) {}
}
