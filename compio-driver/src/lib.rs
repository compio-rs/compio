//! The platform-specified driver.
//! Some types differ by compilation target.

#![cfg_attr(feature = "once_cell_try", feature(once_cell_try))]
#![warn(missing_docs)]

#[cfg(all(
    target_os = "linux",
    not(feature = "io-uring"),
    not(feature = "polling")
))]
compile_error!("You must choose one of these features: [\"io-uring\", \"polling\"]");

use std::{collections::VecDeque, io, time::Duration};

use compio_buf::BufResult;
use slab::Slab;

pub mod op;
#[cfg(unix)]
mod unix;

cfg_if::cfg_if! {
    if #[cfg(target_os = "windows")] {
        #[path = "iocp/mod.rs"]
        mod sys;
    } else if #[cfg(all(target_os = "linux", feature = "io-uring"))] {
        #[path = "iour/mod.rs"]
        mod sys;
    } else if #[cfg(unix)] {
        #[path = "poll/mod.rs"]
        mod sys;
    }
}

pub use sys::*;

#[cfg(target_os = "windows")]
#[macro_export]
#[doc(hidden)]
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
#[macro_export]
#[doc(hidden)]
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
        match $crate::syscall!( $fn ( $($arg, )* )) {
            Ok(fd) => ::std::task::Poll::Ready(Ok(fd as usize)),
            Err(e) if e.kind() == ::std::io::ErrorKind::WouldBlock || e.raw_os_error() == Some(::libc::EINPROGRESS)
                   => ::std::task::Poll::Pending,
            Err(e) => ::std::task::Poll::Ready(Err(e)),
        }
    };
    ($fn: ident ( $($arg: expr),* $(,)* ) or $f:ident($fd:expr)) => {
        match $crate::syscall!( break $fn ( $($arg, )* )) {
            ::std::task::Poll::Pending => Ok($crate::Decision::$f($fd)),
            ::std::task::Poll::Ready(Ok(res)) => Ok($crate::Decision::Completed(res)),
            ::std::task::Poll::Ready(Err(e)) => Err(e),
        }
    };
}

#[macro_export]
#[doc(hidden)]
macro_rules! impl_raw_fd {
    ($t:ty, $inner:ident) => {
        impl $crate::AsRawFd for $t {
            fn as_raw_fd(&self) -> $crate::RawFd {
                self.$inner.as_raw_fd()
            }
        }
        impl $crate::FromRawFd for $t {
            unsafe fn from_raw_fd(fd: $crate::RawFd) -> Self {
                Self {
                    $inner: $crate::FromRawFd::from_raw_fd(fd),
                }
            }
        }
        impl $crate::IntoRawFd for $t {
            fn into_raw_fd(self) -> $crate::RawFd {
                self.$inner.into_raw_fd()
            }
        }
    };
}

/// Low-level actions of completion-based IO.
/// It owns the operations to keep the driver safe.
pub struct Proactor {
    driver: Driver,
    ops: Slab<RawOp>,
    squeue: VecDeque<usize>,
}

impl Proactor {
    /// Create [`Proactor`] with 1024 entries.
    pub fn new() -> io::Result<Self> {
        Self::with_entries(1024)
    }

    /// Create [`Proactor`] with specified entries.
    pub fn with_entries(entries: u32) -> io::Result<Self> {
        Ok(Self {
            driver: Driver::new(entries)?,
            ops: Slab::with_capacity(entries as _),
            squeue: VecDeque::with_capacity(entries as _),
        })
    }

    /// Attach an fd to the driver. It will cause unexpected result to attach
    /// the handle with one driver and push an op to another driver.
    ///
    /// ## Platform specific
    /// * IOCP: it will be attached to the completion port. An fd could only be
    ///   attached to one driver, and could only be attached once, even if you
    ///   `try_clone` it.
    /// * io-uring: it will do nothing and return `Ok(())`.
    /// * polling: it will initialize inner queue and register to the driver. On
    ///   Linux and Android, if the fd is a normal file or a directory, this
    ///   method will do nothing.
    pub fn attach(&mut self, fd: RawFd) -> io::Result<()> {
        self.driver.attach(fd)
    }

    /// Cancel an operation with the pushed user-defined data.
    ///
    /// The cancellation is not reliable. The underlying operation may continue,
    /// but just don't return from [`Proactor::poll`]. Therefore, although an
    /// operation is cancelled, you should not reuse its `user_data`.
    ///
    /// It is well-defined to cancel before polling. If the submitted operation
    /// contains a cancelled user-defined data, the operation will be ignored.
    pub fn cancel(&mut self, user_data: usize) {
        self.driver.cancel(user_data, &mut self.ops);
    }

    /// Push an operation into the driver, and return the unique key, called
    /// user-defined data, associated with it.
    pub fn push(&mut self, op: impl OpCode + 'static) -> usize {
        let entry = self.ops.vacant_entry();
        let user_data = entry.key();
        let op = RawOp::new(user_data, op);
        entry.insert(op);
        self.squeue.push_back(user_data);
        user_data
    }

    /// Poll the driver and get completed entries.
    /// You need to call [`Proactor::pop`] to get the pushed operations.
    pub fn poll(
        &mut self,
        timeout: Option<Duration>,
        entries: &mut impl Extend<Entry>,
    ) -> io::Result<()> {
        let mut iter = std::iter::from_fn(|| self.squeue.pop_front());
        unsafe {
            self.driver
                .poll(timeout, &mut iter, entries, &mut self.ops)?;
        }
        Ok(())
    }

    /// Get the pushed operations from the completion entries.
    pub fn pop<'a>(
        &'a mut self,
        entries: &'a mut impl Iterator<Item = Entry>,
    ) -> impl Iterator<Item = BufResult<usize, Operation>> + 'a {
        std::iter::from_fn(|| {
            entries.next().map(|entry| {
                let op = self
                    .ops
                    .try_remove(entry.user_data())
                    .expect("the entry should be valid");
                let op = Operation::new(op, entry.user_data());
                BufResult(entry.into_result(), op)
            })
        })
    }
}

impl AsRawFd for Proactor {
    fn as_raw_fd(&self) -> RawFd {
        self.driver.as_raw_fd()
    }
}

/// Contains the operation and the user_data.
pub struct Operation {
    op: RawOp,
    user_data: usize,
}

impl Operation {
    pub(crate) fn new(op: RawOp, user_data: usize) -> Self {
        Self { op, user_data }
    }

    #[doc(hidden)]
    pub fn into_inner(self) -> RawOp {
        self.op
    }

    /// Restore the original operation.
    ///
    /// # Safety
    ///
    /// The caller should guarantee that the type is right.
    pub unsafe fn into_op<T: OpCode>(self) -> T {
        self.into_inner().into_inner()
    }

    /// The same user_data when the operation is pushed into the driver.
    pub fn user_data(&self) -> usize {
        self.user_data
    }
}

/// An completed entry returned from kernel.
#[derive(Debug)]
pub struct Entry {
    user_data: usize,
    result: io::Result<usize>,
}

impl Entry {
    pub(crate) fn new(user_data: usize, result: io::Result<usize>) -> Self {
        Self { user_data, result }
    }

    /// The user-defined data returned by [`Proactor::push`].
    pub fn user_data(&self) -> usize {
        self.user_data
    }

    /// The result of the operation.
    pub fn into_result(self) -> io::Result<usize> {
        self.result
    }
}
