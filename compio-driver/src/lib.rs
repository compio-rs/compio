//! The platform-specified driver.
//! Some types differ by compilation target.

#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![cfg_attr(feature = "once_cell_try", feature(once_cell_try))]
#![warn(missing_docs)]

#[cfg(all(
    target_os = "linux",
    not(feature = "io-uring"),
    not(feature = "polling")
))]
compile_error!("You must choose at least one of these features: [\"io-uring\", \"polling\"]");

use std::{io, task::Poll, time::Duration};

use compio_buf::BufResult;
use compio_log::{instrument, trace};
use slab::Slab;

mod key;
pub use key::Key;

pub mod op;
#[cfg(unix)]
#[cfg_attr(docsrs, doc(cfg(all())))]
mod unix;

mod asyncify;
pub use asyncify::*;

mod fd;
pub use fd::*;

cfg_if::cfg_if! {
    if #[cfg(windows)] {
        #[path = "iocp/mod.rs"]
        mod sys;
    } else if #[cfg(all(target_os = "linux", feature = "polling", feature = "io-uring"))] {
        #[path = "fusion/mod.rs"]
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

#[cfg(windows)]
#[macro_export]
#[doc(hidden)]
macro_rules! syscall {
    (BOOL, $e:expr) => {
        $crate::syscall!($e, == 0)
    };
    (SOCKET, $e:expr) => {
        $crate::syscall!($e, != 0)
    };
    (HANDLE, $e:expr) => {
        $crate::syscall!($e, == ::windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE)
    };
    ($e:expr, $op: tt $rhs: expr) => {{
        #[allow(unused_unsafe)]
        let res = unsafe { $e };
        if res $op $rhs {
            Err(::std::io::Error::last_os_error())
        } else {
            Ok(res)
        }
    }};
}

/// Helper macro to execute a system call
#[cfg(unix)]
#[macro_export]
#[doc(hidden)]
macro_rules! syscall {
    (break $e:expr) => {
        match $crate::syscall!($e) {
            Ok(fd) => ::std::task::Poll::Ready(Ok(fd as usize)),
            Err(e) if e.kind() == ::std::io::ErrorKind::WouldBlock || e.raw_os_error() == Some(::libc::EINPROGRESS)
                   => ::std::task::Poll::Pending,
            Err(e) => ::std::task::Poll::Ready(Err(e)),
        }
    };
    ($e:expr, $f:ident($fd:expr)) => {
        match $crate::syscall!(break $e) {
            ::std::task::Poll::Pending => Ok($crate::sys::Decision::$f($fd)),
            ::std::task::Poll::Ready(Ok(res)) => Ok($crate::sys::Decision::Completed(res)),
            ::std::task::Poll::Ready(Err(e)) => Err(e),
        }
    };
    ($e:expr) => {{
        #[allow(unused_unsafe)]
        let res = unsafe { $e };
        if res == -1 {
            Err(::std::io::Error::last_os_error())
        } else {
            Ok(res)
        }
    }};
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
        #[cfg(unix)]
        impl std::os::fd::FromRawFd for $t {
            unsafe fn from_raw_fd(fd: $crate::RawFd) -> Self {
                Self {
                    $inner: std::os::fd::FromRawFd::from_raw_fd(fd),
                }
            }
        }
        impl $crate::ToSharedFd for $t {
            fn to_shared_fd(&self) -> $crate::SharedFd {
                self.$inner.to_shared_fd()
            }
        }
    };
    ($t:ty, $inner:ident,file) => {
        $crate::impl_raw_fd!($t, $inner);
        #[cfg(windows)]
        impl std::os::windows::io::FromRawHandle for $t {
            unsafe fn from_raw_handle(handle: std::os::windows::io::RawHandle) -> Self {
                Self {
                    $inner: std::os::windows::io::FromRawHandle::from_raw_handle(handle),
                }
            }
        }
    };
    ($t:ty, $inner:ident,socket) => {
        $crate::impl_raw_fd!($t, $inner);
        #[cfg(windows)]
        impl std::os::windows::io::FromRawSocket for $t {
            unsafe fn from_raw_socket(sock: std::os::windows::io::RawSocket) -> Self {
                Self {
                    $inner: std::os::windows::io::FromRawSocket::from_raw_socket(sock),
                }
            }
        }
    };
}

/// The return type of [`Proactor::push`].
pub enum PushEntry<K, R> {
    /// The operation is pushed to the submission queue.
    Pending(K),
    /// The operation is ready and returns.
    Ready(R),
}

impl<K, R> PushEntry<K, R> {
    /// Map the [`PushEntry::Pending`] branch.
    pub fn map_pending<L>(self, f: impl FnOnce(K) -> L) -> PushEntry<L, R> {
        match self {
            Self::Pending(k) => PushEntry::Pending(f(k)),
            Self::Ready(r) => PushEntry::Ready(r),
        }
    }

    /// Map the [`PushEntry::Ready`] branch.
    pub fn map_ready<S>(self, f: impl FnOnce(R) -> S) -> PushEntry<K, S> {
        match self {
            Self::Pending(k) => PushEntry::Pending(k),
            Self::Ready(r) => PushEntry::Ready(f(r)),
        }
    }
}

/// Low-level actions of completion-based IO.
/// It owns the operations to keep the driver safe.
pub struct Proactor {
    driver: Driver,
    ops: Slab<RawOp>,
}

impl Proactor {
    /// Create [`Proactor`] with 1024 entries.
    pub fn new() -> io::Result<Self> {
        Self::builder().build()
    }

    /// Create [`ProactorBuilder`] to config the proactor.
    pub fn builder() -> ProactorBuilder {
        ProactorBuilder::new()
    }

    fn with_builder(builder: &ProactorBuilder) -> io::Result<Self> {
        Ok(Self {
            driver: Driver::new(builder)?,
            ops: Slab::with_capacity(builder.capacity as _),
        })
    }

    /// Attach an fd to the driver.
    ///
    /// ## Platform specific
    /// * IOCP: it will be attached to the completion port. An fd could only be
    ///   attached to one driver, and could only be attached once, even if you
    ///   `try_clone` it.
    /// * io-uring & polling: it will do nothing but return `Ok(())`.
    pub fn attach(&mut self, fd: RawFd) -> io::Result<()> {
        self.driver.attach(fd)
    }

    /// Cancel an operation with the pushed user-defined data.
    ///
    /// The cancellation is not reliable. The underlying operation may continue,
    /// but just don't return from [`Proactor::poll`]. Therefore, although an
    /// operation is cancelled, you should not reuse its `user_data`.
    ///
    /// It is *safe* to cancel before polling. If the submitted operation
    /// contains a cancelled user-defined data, the operation will be ignored.
    /// However, to make the operation dropped correctly, you should cancel
    /// after push.
    pub fn cancel(&mut self, user_data: usize) {
        instrument!(compio_log::Level::DEBUG, "cancel", user_data);
        if let Some(op) = self.ops.get_mut(user_data) {
            if op.set_cancelled() {
                // The op is completed.
                trace!("cancel and remove {}", user_data);
                self.ops.remove(user_data);
                return;
            }
        }
        self.driver.cancel(user_data, &mut self.ops);
    }

    /// Push an operation into the driver, and return the unique key, called
    /// user-defined data, associated with it.
    pub fn push<T: OpCode + 'static>(&mut self, op: T) -> PushEntry<Key<T>, BufResult<usize, T>> {
        let entry = self.ops.vacant_entry();
        let user_data = entry.key();
        let op = self.driver.create_op(user_data, op);
        let op = entry.insert(op);
        match self.driver.push(user_data, op) {
            Poll::Pending => PushEntry::Pending(unsafe { Key::new(user_data) }),
            Poll::Ready(res) => {
                let mut op = self.ops.remove(user_data);
                op.set_result(res);
                PushEntry::Ready(unsafe { op.into_inner::<T>() })
            }
        }
    }

    /// Poll the driver and get completed entries.
    /// You need to call [`Proactor::pop`] to get the pushed operations.
    pub fn poll(
        &mut self,
        timeout: Option<Duration>,
        entries: &mut impl Extend<usize>,
    ) -> io::Result<()> {
        unsafe {
            self.driver
                .poll(timeout, OutEntries::new(entries, &mut self.ops))?;
        }
        Ok(())
    }

    /// Get the pushed operations from the completion entries.
    ///
    /// # Panics
    /// This function will panic if the requested operation has not been
    /// completed.
    pub fn pop<T: OpCode>(&mut self, user_data: Key<T>) -> BufResult<usize, T> {
        instrument!(compio_log::Level::DEBUG, "pop", ?user_data);
        let op = self
            .ops
            .try_remove(*user_data)
            .expect("the entry should be valid");
        trace!("poped {}", *user_data);
        // Safety: user cannot create key with safe code, so the type should be correct
        unsafe { op.into_inner::<T>() }
    }

    /// Query if the operation has completed.
    pub fn has_result(&self, user_data: usize) -> bool {
        self.ops
            .get(user_data)
            .map(|op| op.has_result())
            .unwrap_or_default()
    }

    /// Create a notify handle to interrupt the inner driver.
    pub fn handle(&self) -> io::Result<NotifyHandle> {
        self.driver.handle()
    }
}

impl AsRawFd for Proactor {
    fn as_raw_fd(&self) -> RawFd {
        self.driver.as_raw_fd()
    }
}

/// An completed entry returned from kernel.
#[derive(Debug)]
pub(crate) struct Entry {
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

// The output entries need to be marked as `completed`. If an entry has been
// marked as `cancelled`, it will be removed from the registry.
struct OutEntries<'a, 'b, E> {
    entries: &'b mut E,
    registry: &'a mut Slab<RawOp>,
}

impl<'a, 'b, E> OutEntries<'a, 'b, E> {
    pub fn new(entries: &'b mut E, registry: &'a mut Slab<RawOp>) -> Self {
        Self { entries, registry }
    }

    #[allow(dead_code)]
    pub fn registry(&mut self) -> &mut Slab<RawOp> {
        self.registry
    }
}

impl<E: Extend<usize>> Extend<Entry> for OutEntries<'_, '_, E> {
    fn extend<T: IntoIterator<Item = Entry>>(&mut self, iter: T) {
        self.entries.extend(iter.into_iter().filter_map(|e| {
            let user_data = e.user_data();
            if self.registry[user_data].set_result(e.into_result()) {
                self.registry.remove(user_data);
                None
            } else {
                Some(user_data)
            }
        }))
    }
}

#[derive(Debug, Clone)]
enum ThreadPoolBuilder {
    Create { limit: usize, recv_limit: Duration },
    Reuse(AsyncifyPool),
}

impl Default for ThreadPoolBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ThreadPoolBuilder {
    pub fn new() -> Self {
        Self::Create {
            limit: 256,
            recv_limit: Duration::from_secs(60),
        }
    }

    pub fn create_or_reuse(&self) -> AsyncifyPool {
        match self {
            Self::Create { limit, recv_limit } => AsyncifyPool::new(*limit, *recv_limit),
            Self::Reuse(pool) => pool.clone(),
        }
    }
}

/// Builder for [`Proactor`].
#[derive(Debug, Clone)]
pub struct ProactorBuilder {
    capacity: u32,
    pool_builder: ThreadPoolBuilder,
}

impl Default for ProactorBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ProactorBuilder {
    /// Create the builder with default config.
    pub fn new() -> Self {
        Self {
            capacity: 1024,
            pool_builder: ThreadPoolBuilder::new(),
        }
    }

    /// Set the capacity of the inner event queue or submission queue, if
    /// exists. The default value is 1024.
    pub fn capacity(&mut self, capacity: u32) -> &mut Self {
        self.capacity = capacity;
        self
    }

    /// Set the thread number limit of the inner thread pool, if exists. The
    /// default value is 256.
    ///
    /// It will be ignored if `reuse_thread_pool` is set.
    pub fn thread_pool_limit(&mut self, value: usize) -> &mut Self {
        if let ThreadPoolBuilder::Create { limit, .. } = &mut self.pool_builder {
            *limit = value;
        }
        self
    }

    /// Set the waiting timeout of the inner thread, if exists. The default is
    /// 60 seconds.
    ///
    /// It will be ignored if `reuse_thread_pool` is set.
    pub fn thread_pool_recv_timeout(&mut self, timeout: Duration) -> &mut Self {
        if let ThreadPoolBuilder::Create { recv_limit, .. } = &mut self.pool_builder {
            *recv_limit = timeout;
        }
        self
    }

    /// Set to reuse an existing [`AsyncifyPool`] in this proactor.
    pub fn reuse_thread_pool(&mut self, pool: AsyncifyPool) -> &mut Self {
        self.pool_builder = ThreadPoolBuilder::Reuse(pool);
        self
    }

    /// Force reuse the thread pool for each proactor created by this builder,
    /// even `reuse_thread_pool` is not set.
    pub fn force_reuse_thread_pool(&mut self) -> &mut Self {
        self.reuse_thread_pool(self.create_or_get_thread_pool());
        self
    }

    /// Create or reuse the thread pool from the config.
    pub fn create_or_get_thread_pool(&self) -> AsyncifyPool {
        self.pool_builder.create_or_reuse()
    }

    /// Build the [`Proactor`].
    pub fn build(&self) -> io::Result<Proactor> {
        Proactor::with_builder(self)
    }
}
