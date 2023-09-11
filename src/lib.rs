//!
#![doc = include_str!("../README.md")]
#![cfg_attr(feature = "read_buf", feature(read_buf))]
#![cfg_attr(feature = "lazy_cell", feature(lazy_cell))]
#![cfg_attr(feature = "once_cell_try", feature(once_cell_try))]
#![warn(missing_docs)]

pub mod buf;
pub mod driver;
pub mod fs;
pub mod net;
pub mod op;

#[cfg(target_os = "windows")]
pub mod named_pipe;

#[cfg(feature = "event")]
pub mod event;
#[cfg(feature = "runtime")]
mod key;
#[cfg(feature = "runtime")]
pub(crate) use key::Key;
#[cfg(feature = "signal")]
pub mod signal;
#[cfg(feature = "runtime")]
pub mod task;
#[cfg(feature = "time")]
pub mod time;

/// A specialized `Result` type for operations with buffers.
///
/// This type is used as a return value for asynchronous IOCP methods that
/// require passing ownership of a buffer to the runtime. When the operation
/// completes, the buffer is returned whether or not the operation completed
/// successfully.
pub type BufResult<T, B> = (std::io::Result<T>, B);

macro_rules! impl_registered_fd {
    ($t:ty, $inner:ident) => {
        impl crate::driver::AsRegisteredFd for $t {
            fn as_registered_fd(&self) -> crate::driver::RegisteredFd {
                self.$inner.as_registered_fd()
            }
        }
        impl crate::driver::AsRawFd for $t {
            fn as_raw_fd(&self) -> crate::driver::RawFd {
                self.$inner.as_raw_fd()
            }
        }
    };
}

pub(crate) use impl_registered_fd;
