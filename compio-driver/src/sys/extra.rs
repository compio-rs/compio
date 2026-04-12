use std::{fmt::Debug, io};

use super::*;

/// Platform-specific extra data associated with a driver instance.
///
/// It can be used to set options for or get extra data from I/O operations.
#[repr(transparent)]
pub struct Extra(pub(super) imp::Extra);

impl Debug for Extra {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Debug::fmt(&self.0, f)
    }
}

impl<I: Into<imp::Extra>> From<I> for Extra {
    fn from(inner: I) -> Self {
        Self::new(inner.into())
    }
}

impl Extra {
    pub(in crate::sys) fn new(inner: imp::Extra) -> Self {
        Self(inner)
    }

    /// Set the personality, returning the modified Extra.
    ///
    /// This is a no-op when not using `io_uring` driver.
    pub fn with_personality(mut self, personality: u16) -> Self {
        self.set_personality(personality);
        self
    }

    /// Set the personality for this operation.
    ///
    /// This is a no-op when not using `io_uring` driver.
    pub fn set_personality(&mut self, personality: u16) {
        #[cfg(io_uring)]
        if let Some(extra) = self.try_as_iour_mut() {
            extra.set_personality(personality);
        }
        #[cfg(not(io_uring))]
        let _ = personality;
    }

    /// Get the personality for this operation.
    ///
    /// If the personality was not set with [`set_personality`] or the platform
    /// does not support it, returns [`None`].
    ///
    /// [`set_personality`]: Extra::set_personality
    pub fn get_personality(&self) -> Option<u16> {
        #[cfg(io_uring)]
        if let Some(extra) = self.try_as_iour() {
            extra.get_personality()
        } else {
            None
        }
        #[cfg(not(io_uring))]
        None
    }

    /// Try to get the buffer ID associated with this operation.
    ///
    /// # Behavior
    ///
    /// This is only supported on `io_uring` drivers, in which the driver will
    /// try to extract `buffer_id` returned by the kernel as a part of `flags`.
    /// If the id cannot be extracted from the flag, an [`InvalidInput`]
    /// [`io::Error`] will be returned. On other platforms, this will always
    /// return [`Unsupported`] error.
    ///
    /// [`InvalidInput`]: io::ErrorKind::InvalidInput
    /// [`Unsupported`]: io::ErrorKind::Unsupported
    pub fn buffer_id(&self) -> io::Result<u16> {
        #[cfg(io_uring)]
        {
            if let Some(extra) = self.try_as_iour() {
                extra
                    .buffer_id()
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "flags are invalid"))
            } else {
                Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "buffer_id is only supported on io_uring driver",
                ))
            }
        }
        #[cfg(not(io_uring))]
        {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "buffer_id is only supported on io_uring driver",
            ))
        }
    }

    /// Checks whether the underlying socket has more data to be read.
    ///
    /// # Behaviour
    ///
    /// This method must be used only on the flags for any of the `receive`
    /// variants supported by `IO_URING`.
    /// The driver will try to check whether the `IORING_CQE_F_SOCK_NONEMPTY`
    /// flag was set by the kernel for the CQE. On other platforms, this will
    /// always return the [`Unsupported`] error.
    ///
    /// [`Unsupported`]: io::ErrorKind::Unsupported
    pub fn sock_nonempty(&self) -> io::Result<bool> {
        #[cfg(io_uring)]
        {
            if let Some(extra) = self.try_as_iour() {
                Ok(extra.sock_nonempty())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "IORING_CQE_F_SOCK_NONEMPTY flag is available only on the io_uring driver",
                ))
            }
        }
        #[cfg(not(io_uring))]
        {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "IORING_CQE_F_SOCK_NONEMPTY flag is available only on the io_uring driver",
            ))
        }
    }
}
