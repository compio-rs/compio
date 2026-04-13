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
        Self(inner.into())
    }
}

impl Extra {
    iour_only! {
        /// Checks whether this completion reports a notification (2nd CQE returned for a zerocopy op).
        ///
        /// # Behaviour
        ///
        /// This is only supported on `io_uring` drivers, in which the driver will
        /// check whether the `IORING_CQE_F_NOTIF` flag was set by the kernel for
        /// the CQE. On other platforms, this will always return the
        /// [`Unsupported`] error.
        ///
        /// [`Unsupported`]: io::ErrorKind::Unsupported
        get fn is_notification(&self) -> io::Result<bool> = |extra| Ok(extra.is_notification());

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
        get fn buffer_id(&self) -> io::Result<u16> =
            |extra| extra
                .buffer_id()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Buffer id was not set"));


        /// Get the personality for this operation.
        ///
        /// # Behavior
        ///
        /// - If the driver is not `io_uring`, return [`Unsupported`] error,
        /// - If the personality was not set with [`set_personality`], return `Ok(None)`
        /// - Otherwise, return `Ok(Some(personality))`
        ///
        /// [`Unsupported`]: io::ErrorKind::Unsupported
        /// [`set_personality`]: Extra::set_personality
        get fn get_personality(&self) -> io::Result<Option<u16>> = |extra| Ok(extra.get_personality());

        /// Checks whether the underlying socket has more data to be read.
        ///
        /// # Behaviour
        ///
        /// This method must be used only on the flags for any of the `receive`
        /// variants supported by `IO_URING`. The driver will try to check whether
        /// the `IORING_CQE_F_SOCK_NONEMPTY` flag was set by the kernel for the CQE.
        /// On other platforms, this will always return the [`Unsupported`] error.
        ///
        /// [`Unsupported`]: io::ErrorKind::Unsupported
        get fn sock_nonempty(&self) -> io::Result<bool> = |extra| Ok(extra.sock_nonempty());

        /// Set the `IOSQE_IO_DRAIN` flag for this operation.
        ///
        /// This ensures that this operation won't start until all previously submitted operations complete.
        ///
        /// See [`io_uring_sqe_set_flags(3)`] for more details.
        ///
        /// [`io_uring_sqe_set_flags(3)`]: https://man7.org/linux/man-pages/man3/io_uring_sqe_set_flags.3.html
        set fn drain(&mut self) = |extra| extra.set_drain();

        /// Set the `IOSQE_IO_LINK` flag for this operation.
        ///
        /// This links this operation with the next one. The next operation will not start until this operation
        /// completed successfully.
        ///
        /// See [`io_uring_sqe_set_flags(3)`] for more details.
        ///
        /// [`io_uring_sqe_set_flags(3)`]: https://man7.org/linux/man-pages/man3/io_uring_sqe_set_flags.3.html
        set fn link(&mut self) = |extra| extra.set_link();

        /// Set the `IOSQE_IO_HARDLINK` flag for this operation.
        ///
        /// Like link, but the next operation will execute regardless of this operation's result.
        ///
        /// See [`io_uring_sqe_set_flags(3)`] for more details.
        ///
        /// [`io_uring_sqe_set_flags(3)`]: https://man7.org/linux/man-pages/man3/io_uring_sqe_set_flags.3.html
        set fn hardlink(&mut self) = |extra| extra.set_hardlink();

        /// Set the personality for this operation.
        ///
        /// A personality represents a set of credentials (uid, gid, etc.) that will be used for this operation.
        ///
        /// The personality can be retrieved with [`Proactor::register_personality`].
        ///
        /// [`Proactor::register_personality`]: crate::Proactor::register_personality
        set fn personality(&mut self, personality: u16) = |extra| extra.set_personality(personality);
    }
}

macro_rules! iour_only {
    {} => {};
    {
        $(#[$doc:meta])*
        get fn $fn:ident(&$this:ident) -> io::Result<$ret:ty> = |$extra:ident| $body:expr;
        $($rest:tt)*
    } => {
        $(#[$doc])*
        pub fn $fn (&$this) -> io::Result<$ret> {
            const UNSUPPORTED: &str = concat!(stringify!($fn), " is only supported on the io_uring driver");
            #[cfg(io_uring)]
            if let Some($extra) = $this.try_as_iour() {
                $body
            } else {
                Err(io::Error::new(io::ErrorKind::Unsupported, UNSUPPORTED))
            }
            #[cfg(not(io_uring))]
            Err(io::Error::new(io::ErrorKind::Unsupported, UNSUPPORTED))
        }
        iour_only!($($rest)*);
    };
    {
        $(#[$doc:meta])*
        set fn $val:ident(&mut $this:ident $(, $arg:ident: $arg_ty:ty)*) = |$extra:ident| $body:expr;
        $($rest:tt)*
    } => {
        paste::paste! {
            $(#[$doc])*
            #[doc = " This is a no-op when not using `io_uring` driver."]
            pub fn [<set_ $val>] (&mut $this $(, $arg: $arg_ty)*) {
                #[cfg(io_uring)]
                if let Some($extra) = $this.try_as_iour_mut() {
                    $body
                }
                #[cfg(not(io_uring))]
                {$(let _ = $arg;)*}
            }

            #[doc = concat!("Call [`set_", stringify!($val), "`] and return the modified `Extra`.")]
            #[doc = ""]
            #[doc = concat!("[`set_", stringify!($val), "`]: Self::set_", stringify!($val))]
            pub fn [<with_ $val>] (mut $this $(, $arg: $arg_ty)*) -> Self {
                $this.[<set_ $val>]($($arg),*);
                $this
            }
        }
        iour_only!($($rest)*);
    };
}

use iour_only;
