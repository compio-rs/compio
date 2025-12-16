cfg_if::cfg_if! {
    if #[cfg(windows)] {
        mod iocp;
        use iocp as imp;
    } else if #[cfg(fusion)] {
        mod fusion;
        mod poll;
        mod iour;

        use fusion as imp;
    } else if #[cfg(io_uring)] {
        mod iour;
        use iour as imp;
    } else if #[cfg(all(target_os = "linux", not(feature = "polling")))] {
        mod stub;
        use stub as imp;
    } else if #[cfg(unix)] {
        mod poll;
        use poll as imp;
    }
}

/// Platform-specific extra data associated with a driver instance.
///
/// This is currently only useful for `io_uring` drivers to store the flags
/// returned by kernel for retrieving buffers from buffer pool.
#[repr(transparent)]
#[derive(Default)]
pub struct Extra(imp::Extra);

impl Extra {
    pub(crate) fn new(driver: RawFd) -> Self {
        Self(imp::Extra::new(driver))
    }

    /// Try to get the buffer ID associated with this operation.
    ///
    /// # Behavior
    ///
    /// This is only supported on `io_uring` drivers, in which the driver will
    /// try to extract `buffer_id` returned by the kernel as a part of `flags`.
    /// If the id cannot be extracted from the flag, an [`InvalidInput`]
    /// [`io::Error`] will be returned. On other platforms, this will always
    /// return `Ok(0)`.
    ///
    /// [`InvalidInput`]: io::ErrorKind::InvalidInput
    pub fn buffer_id(&self) -> io::Result<u16> {
        #[cfg(io_uring)]
        {
            self.0
                .buffer_id()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "flags are invalid"))
        }
        #[cfg(not(io_uring))]
        {
            // On other platforms, buffer IDs are not supported nor used, so it's okay to
            // return `Ok(0)`.
            Ok(0)
        }
    }
}

#[cfg(aio)]
pub(crate) mod aio {
    pub use libc::aiocb;

    pub fn new_aiocb() -> aiocb {
        unsafe { std::mem::zeroed() }
    }
}

#[cfg(not(aio))]
pub(crate) mod aio {
    #[allow(non_camel_case_types)]
    pub type aiocb = ();

    pub fn new_aiocb() -> aiocb {}
}

use std::io;

pub use imp::*;

#[cfg(unix)]
mod unix_op;
