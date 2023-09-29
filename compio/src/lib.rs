//!
#![doc = include_str!("../../README.md")]
#![cfg_attr(feature = "allocator_api", feature(allocator_api))]
#![cfg_attr(feature = "lazy_cell", feature(lazy_cell))]
#![cfg_attr(feature = "once_cell_try", feature(once_cell_try))]
#![cfg_attr(feature = "read_buf", feature(read_buf))]
#![warn(missing_docs)]

pub mod fs;
pub mod net;

pub use buf::BufResult;
pub use compio_buf as buf;

#[cfg(target_os = "windows")]
pub mod named_pipe;

#[cfg(feature = "event")]
pub mod event;
#[cfg(feature = "runtime")]
mod key;
#[cfg(feature = "runtime")]
pub(crate) use key::Key;
#[cfg(feature = "runtime")]
mod attacher;
#[cfg(feature = "runtime")]
pub(crate) use attacher::Attacher;
#[cfg(feature = "signal")]
pub mod signal;
#[cfg(feature = "macros")]
pub use compio_macros::*;
#[cfg(feature = "runtime")]
pub mod task;
#[cfg(feature = "time")]
pub mod time;

#[cfg(feature = "runtime")]
macro_rules! buf_try {
    ($e:expr) => {{
        match $e {
            (Ok(res), buf) => (res, buf),
            (Err(e), buf) => return (Err(e), buf),
        }
    }};
    ($e:expr, $b:expr) => {{
        let buf = $b;
        match $e {
            Ok(res) => (res, buf),
            Err(e) => return (Err(e), buf),
        }
    }};
}

#[cfg(feature = "runtime")]
pub(crate) use buf_try;

macro_rules! impl_raw_fd {
    ($t:ty, $inner:ident $(, $attacher:ident)?) => {
        impl crate::driver::AsRawFd for $t {
            fn as_raw_fd(&self) -> crate::driver::RawFd {
                self.$inner.as_raw_fd()
            }
        }
        impl crate::driver::FromRawFd for $t {
            unsafe fn from_raw_fd(fd: crate::driver::RawFd) -> Self {
                Self {
                    $inner: crate::driver::FromRawFd::from_raw_fd(fd),
                    $(
                        #[cfg(feature = "runtime")]
                        $attacher: crate::Attacher::new(),
                    )?
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
