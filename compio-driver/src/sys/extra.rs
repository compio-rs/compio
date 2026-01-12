use std::{fmt::Debug, io};

use super::*;

/// Platform-specific extra data associated with a driver instance.
///
/// - On Windows, it holds the `OVERLAPPED` buffer and a pointer to the driver.
/// - On Linux with `io_uring`, it holds the flags returned by kernel.
/// - On other platforms, it is empty.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct Extra(pub(super) imp::Extra);

impl Debug for Extra {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Extra").field("sys", &"<...>").finish()
    }
}

impl<I: Into<imp::Extra>> From<I> for Extra {
    fn from(inner: I) -> Self {
        Self::new(inner.into())
    }
}

impl Extra {
    pub(super) fn new(inner: imp::Extra) -> Self {
        Self(inner)
    }

    #[cfg(io_uring)]
    pub(super) fn as_iour(&self) -> Option<&iour::Extra> {
        #[cfg(fusion)]
        {
            if let imp::Extra::IoUring(extra) = &self.0 {
                Some(extra)
            } else {
                None
            }
        }
        #[cfg(not(fusion))]
        self.0
    }

    #[cfg(io_uring)]
    pub(super) fn as_iour_mut(&mut self) -> Option<&mut iour::Extra> {
        #[cfg(fusion)]
        {
            if let imp::Extra::IoUring(extra) = &mut self.0 {
                Some(extra)
            } else {
                None
            }
        }
        #[cfg(not(fusion))]
        self.0
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
            if let Some(extra) = self.as_iour() {
                extra
                    .buffer_id()
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "flags are invalid"))
            } else {
                Ok(0)
            }
        }
        #[cfg(not(io_uring))]
        {
            // On other platforms, buffer IDs are not supported nor used, so it's okay to
            // return `Ok(0)`.
            Ok(0)
        }
    }
}
