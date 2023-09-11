//! Registered file descriptor interface with default platform independent
//! implementation

/// Registered file descriptor is valid only for owning thread
///
///
/// It's not `Send` and not `Sync`
use std::{io, marker::PhantomData};

use bitvec::prelude::BitSlice;

use crate::driver::{error::Error, RawFd};

/// Registered fd is owned by a single thread
///
/// Concurrent operations could copy it.
/// If previously registered fd has become unregistered subsequent registered fd
/// usage could result in IO error
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RegisteredFd {
    // 31 bit fd idx
    fd_idx: u32,
    _nor_send_nor_sync: PhantomData<*const ()>,
}

impl RegisteredFd {
    /// Construct registered file descriptor from the chosen offset index
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

/// Returns registered file descriptor associated with the type
pub trait AsRegisteredFd {
    /// Returns registered file descriptor associated with the type
    fn as_registered_fd(&self) -> RegisteredFd;
}

/// Used to skip update of registered file descriptor
pub const SKIP_UPDATE: RawFd = -2 as RawFd;
/// Removes fd from the array of registered file descriptors
pub const UNREGISTERED: RawFd = -1 as RawFd;

/// Public registration API
pub trait RegisteredFileDescriptors: RegisteredFileAllocator {
    /// Selects free registered fd
    ///
    /// It shifts registered fd search offset forward with wrapping to zero
    /// offset
    ///
    /// Returns IOError if there is no registered fd available
    fn reserve_free_registered_fd(&mut self) -> io::Result<RegisteredFd> {
        <Self as RegisteredFileAllocator>::reserve_free_registered_fd(self)
    }

    /// Replaces or unregisters a single file descriptor
    ///
    /// Use `UNREGISTERED` value to unregister and `SKIP_UPDATE` to skip file
    /// update in the corresponding slice slots On Linux will block until
    /// all inflight operations will finish
    fn register_fd(&mut self, registered_fd: RegisteredFd, fd: RawFd) -> io::Result<usize> {
        let offset = u32::from(registered_fd);
        <Self as RegisteredFileDescriptors>::register_files_update(self, offset, &[fd])
    }

    /// Replaces or unregisters file descriptors
    ///
    /// Use `UNREGISTERED` value to unregister and `SKIP_UPDATE` to skip file
    /// update in the corresponding slice slots On Linux will block until
    /// all inflight operations will finish
    fn register_files_update(&mut self, offset: u32, fds: &[RawFd]) -> io::Result<usize>;

    // /// Replace registered files asynchronously
    // ///
    // /// The owned fds slice with the underlying allocation will be returned by a
    // /// runtime method when operation is completed
    // fn push_register_files_update(
    //     &self,
    //     offset: u32,
    //     fds: Slice<Box<[RawFd]>>,
    //     user_data: usize,
    // ) -> io::Result<()>;
}

/// Tracks and allocates registered file descriptors
pub trait RegisteredFileAllocator {
    // bit slice of registered fds
    fn registered_bit_slice(&mut self) -> &BitSlice;

    fn registered_bit_slice_mut(&mut self) -> &mut BitSlice;

    // where to start the next search for free registered fd
    fn registered_fd_search_from(&self) -> u32;

    fn registered_fd_search_from_mut(&mut self) -> &mut u32;

    fn reserve_free_registered_fd(&mut self) -> io::Result<RegisteredFd> {
        let search_from = self.registered_fd_search_from();

        let (fd_idx_usize, len) = {
            let all_registered = self.registered_bit_slice();
            let len = all_registered.len();
            let offset = search_from as usize;
            // first search after last_registered
            let maybe_fdx = if let Some(subslice) = all_registered.get(offset..) {
                if let Some(position) = subslice.first_zero() {
                    let fd_idx_usize = offset + position;
                    Some(fd_idx_usize)
                } else {
                    None
                }
            } else {
                unreachable!("offset is in range")
            };
            if let Some(fdx) = maybe_fdx {
                (fdx, len)
            } else {
                // try to search before last_registered
                if let Some(subslice) = all_registered.get(..offset) {
                    if let Some(position) = subslice.first_zero() {
                        (position, len)
                    } else {
                        return Err(io::Error::from(Error::NoFreeRegisteredFiles));
                    }
                } else {
                    unreachable!("offset is wrapped on slice end")
                }
            }
        };
        self.bump_registered_fd_search_from(fd_idx_usize, len);
        let fd_idx = u32::try_from(fd_idx_usize).expect("in u32 range");
        return Ok(RegisteredFd::new(fd_idx));
    }

    #[inline]
    fn bump_registered_fd_search_from(&mut self, last_used_idx: usize, slice_len: usize) {
        let search_offset = last_used_idx + 1;
        *self.registered_fd_search_from_mut() = u32::try_from(if search_offset < slice_len {
            // wrapping search offset to zero
            0
        } else {
            search_offset
        })
        .expect("search offset in u32 range");
    }

    fn register_files_update(&mut self, offset: u32, fds: &[RawFd]) -> io::Result<usize> {
        let offset_usize = offset as usize;
        if let Some(subslice) = self
            .registered_bit_slice_mut()
            .get_mut(offset_usize..offset_usize + fds.len())
        {
            subslice
                .iter_mut()
                .zip(fds)
                .for_each(|(bit_proxy, fd)| match *fd {
                    UNREGISTERED => bit_proxy.commit(false),
                    SKIP_UPDATE => {}
                    _ => bit_proxy.commit(true),
                });
            Ok(subslice.len())
        } else {
            Err(io::Error::from(Error::FilesOutOfRange))
        }
    }
}

// Platforms that don't have system provided file descriptor registry use
// driver emulated one
#[doc(hidden)]
pub(super) trait FDRegistry {
    fn registered_files(&self) -> &[RawFd];
    fn registered_files_mut(&mut self) -> &mut [RawFd];

    // register file descriptor synchronously
    fn register_files_update(&mut self, offset: u32, fds: &[RawFd]) -> io::Result<usize> {
        let start = offset as usize;
        let end = start + fds.len();
        if let Some(files) = self.registered_files_mut().get_mut(start..end) {
            files
                .iter_mut()
                .zip(fds.iter())
                .for_each(|(f, update)| *f = *update);
            Ok(files.len())
        } else {
            Err(io::Error::from(Error::FilesOutOfRange))
        }
    }

    /// Get raw file descriptor for the given registered descriptor.
    ///
    /// Will panic if the provided registered fd is outside of the range of
    /// registration files
    fn get_raw_fd(&self, registered_fd: RegisteredFd) -> RawFd {
        self.registered_files()[u32::from(registered_fd) as usize]
    }
}
