//! Registered file descriptor interface with default platform independent
//! implementation

/// Registered file descriptor is valid only for owning thread
///
///
/// It's not `Send` and not `Sync`
use std::{
    io::{self},
    marker::PhantomData,
    os::fd::RawFd,
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
    /// Value used before file descriptor is registered
    pub const UNREGISTERED: Self = Self::new(u32::MAX);

    /// Construct registered file descriptor from the provided offset index
    pub const fn new(fd_idx: u32) -> Self {
        Self {
            fd_idx,
            _nor_send_nor_sync: PhantomData,
        }
    }
}

impl From<RegisteredFd> for u32 {
    fn from(other: RegisteredFd) -> u32 {
        other.fd_idx
    }
}

/// Update to registered fd array
#[repr(transparent)]
pub struct RegisteredFdUpdate(RawFd);

impl RegisteredFdUpdate {
    /// Used to skip update of registered file descriptor
    pub const SKIP_UPDATE: RegisteredFdUpdate = RegisteredFdUpdate(-2 as RawFd);
    /// Removes fd from the array of registered file descriptors
    pub const UNREGISTER: RegisteredFdUpdate = RegisteredFdUpdate(-1 as RawFd);

    /// Replaces on registered fd by another
    pub const fn replace(replacement: RawFd) -> Self {
        Self(replacement)
    }
}

/// Public registration API
pub trait RegisteredFileDescriptors: Poller {
    /// Register atached files synchronously
    ///
    /// On Linux will block until all inflight operations will finish
    fn register_attached_files(&self) -> io::Result<()>;
    // /// Register files synchronously
    // ///
    // /// On Linux will block until all inflight operations will finish
    // // fn register_raw_files(&self, fds: &[RawFd]) -> io::Result<()>;
    // /// Replace registered files synchronously
    // ///
    // /// On Linux will block until all inflight operations will finish
    // fn register_files_update(&self, offset: u32, fds: &[RegisteredFdUpdate]) ->
    // io::Result<()>;

    // /// Replace registered files asynchronously
    // ///
    // /// The owned fds slice with the underlying allocation will be returned by a
    // /// runtime method when operation is completed
    // fn push_register_files_update(
    //     &self,
    //     offset: u32,
    //     fds: Slice<Box<[RegisteredFdUpdate]>>,
    //     user_data: usize,
    // ) -> io::Result<()>;
}

// Internal registration API emulated by driver
#[doc(hidden)]
pub(super) trait DriverRegisteredFileDescriptors: Poller {
    // reference to registered files slice
    fn registered_files(&self) -> &[RawFd];
    // mutable reference to registered files slice
    fn registered_files_mut(&self) -> &mut [RawFd];

    fn register_attached_files(&self) -> io::Result<()> {
        // by default registered_fd index is written during attachment
        Ok(())
    }

    fn register_files_update(&self, _offset: u32, _fds: &[RegisteredFdUpdate]) -> io::Result<()> {
        unimplemented!()
    }
    fn push_register_files_update(
        &self,
        _offset: u32,
        _fds: Slice<Box<[RegisteredFdUpdate]>>,
        _user_data: usize,
    ) -> io::Result<()> {
        unimplemented!()
    }
}
