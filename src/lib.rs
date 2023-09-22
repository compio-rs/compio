//!
#![doc = include_str!("../README.md")]
#![cfg_attr(feature = "allocator_api", feature(allocator_api))]
#![cfg_attr(feature = "lazy_cell", feature(lazy_cell))]
#![cfg_attr(feature = "once_cell_try", feature(once_cell_try))]
#![cfg_attr(feature = "read_buf", feature(read_buf))]
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
#[cfg(feature = "runtime")]
mod attacher;
#[cfg(feature = "runtime")]
pub(crate) use attacher::Attacher;
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

#[cfg(target_os = "windows")]
macro_rules! syscall {
    ($fn: ident ( $($arg: expr),* $(,)* ), $op: tt $rhs: expr) => {{
        #[allow(unused_unsafe)]
        let res = unsafe { $fn($($arg, )*) };
        if res $op $rhs {
            Err(::std::io::Error::last_os_error())
        } else {
            Ok(res)
        }
    }};
    (BOOL, $fn: ident ( $($arg: expr),* $(,)* )) => {
        $crate::syscall!($fn($($arg, )*), == 0)
    };
    (SOCKET, $fn: ident ( $($arg: expr),* $(,)* )) => {
        $crate::syscall!($fn($($arg, )*), != 0)
    };
    (HANDLE, $fn: ident ( $($arg: expr),* $(,)* )) => {
        $crate::syscall!($fn($($arg, )*), == ::windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE)
    };
}

/// Helper macro to execute a system call
#[cfg(unix)]
#[allow(unused_macros)]
macro_rules! syscall {
    ($fn: ident ( $($arg: expr),* $(,)* ) ) => {{
        #[allow(unused_unsafe)]
        let res = unsafe { ::libc::$fn($($arg, )*) };
        if res == -1 {
            Err(::std::io::Error::last_os_error())
        } else {
            Ok(res)
        }
    }};
    // The below branches are used by polling driver.
    (break $fn: ident ( $($arg: expr),* $(,)* )) => {
        $crate::syscall!( $fn ( $($arg, )* )).map(
            |res| ::std::ops::ControlFlow::Break(res as usize)
        )
    };
    ($fn: ident ( $($arg: expr),* $(,)* ) or $f:ident($fd:expr)) => {
        match $crate::syscall!( $fn ( $($arg, )* )) {
            Ok(fd) => Ok($crate::driver::Decision::Completed(fd as usize)),
            Err(e) if e.kind() == ::std::io::ErrorKind::WouldBlock || e.raw_os_error() == Some(::libc::EINPROGRESS)
                   => Ok($crate::driver::Decision::$f($fd)),
            Err(e) => Err(e),
        }
    };
}

#[allow(unused_imports)]
pub(crate) use syscall;

#[cfg(not(feature = "allocator_api"))]
macro_rules! vec_alloc {
    ($t:ident, $a:ident) => {
        Vec<$t>
    };
}

#[cfg(feature = "allocator_api")]
macro_rules! vec_alloc {
    ($t:ident, $a:ident) => {
        Vec<$t, $a>
    };
}

pub(crate) use vec_alloc;
