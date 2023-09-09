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

cfg_if::cfg_if! {
    if #[cfg(feature = "runtime")] {
        pub mod task;
        mod key;
        use key::Key;
    }
}

#[cfg(target_os = "windows")]
pub mod named_pipe;

#[cfg(feature = "signal")]
pub mod signal;

#[cfg(feature = "time")]
pub mod time;

/// A specialized `Result` type for operations with buffers.
///
/// This type is used as a return value for asynchronous IOCP methods that
/// require passing ownership of a buffer to the runtime. When the operation
/// completes, the buffer is returned whether or not the operation completed
/// successfully.
pub type BufResult<T, B> = (std::io::Result<T>, B);

macro_rules! impl_raw_fd {
    ($t:ty, $inner:ident) => {
        impl crate::driver::AsRawFd for $t {
            fn as_raw_fd(&self) -> crate::driver::RawFd {
                self.$inner.as_raw_fd()
            }
        }
        impl crate::driver::FromRawFd for $t {
            unsafe fn from_raw_fd(fd: crate::driver::RawFd) -> Self {
                Self {
                    $inner: crate::driver::FromRawFd::from_raw_fd(fd),
                }
            }
        }
        impl crate::driver::IntoRawFd for $t {
            fn into_raw_fd(self) -> crate::driver::RawFd {
                self.$inner.into_raw_fd()
            }
        }
    };
}

pub(crate) use impl_raw_fd;
