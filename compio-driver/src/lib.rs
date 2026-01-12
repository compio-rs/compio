//! Platform-specific drivers.
//!
//! Some types differ by compilation target.

#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(feature = "once_cell_try", feature(once_cell_try))]
#![warn(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]

use std::{
    io,
    task::{Poll, Waker},
    time::Duration,
};

use compio_buf::BufResult;
use compio_log::instrument;

mod key;
pub use key::Key;

mod asyncify;
pub use asyncify::*;

pub mod op;

mod fd;
pub use fd::*;

mod driver_type;
pub use driver_type::*;

mod buffer_pool;
pub use buffer_pool::*;

mod sys;
pub use sys::*;

use crate::key::ErasedKey;

mod sys_slice;

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
        loop {
            match $crate::syscall!($e) {
                Ok(fd) => break ::std::task::Poll::Ready(Ok(fd as usize)),
                Err(e) if e.kind() == ::std::io::ErrorKind::WouldBlock || e.raw_os_error() == Some(::libc::EINPROGRESS)
                    => break ::std::task::Poll::Pending,
                Err(e) if e.kind() == ::std::io::ErrorKind::Interrupted => {},
                Err(e) => break ::std::task::Poll::Ready(Err(e)),
            }
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
    ($t:ty, $it:ty, $inner:ident) => {
        impl $crate::AsRawFd for $t {
            fn as_raw_fd(&self) -> $crate::RawFd {
                self.$inner.as_raw_fd()
            }
        }
        #[cfg(unix)]
        impl std::os::fd::AsFd for $t {
            fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
                self.$inner.as_fd()
            }
        }
        #[cfg(unix)]
        impl std::os::fd::FromRawFd for $t {
            unsafe fn from_raw_fd(fd: $crate::RawFd) -> Self {
                Self {
                    $inner: unsafe { std::os::fd::FromRawFd::from_raw_fd(fd) },
                }
            }
        }
        impl $crate::ToSharedFd<$it> for $t {
            fn to_shared_fd(&self) -> $crate::SharedFd<$it> {
                self.$inner.to_shared_fd()
            }
        }
    };
    ($t:ty, $it:ty, $inner:ident,file) => {
        $crate::impl_raw_fd!($t, $it, $inner);
        #[cfg(windows)]
        impl std::os::windows::io::FromRawHandle for $t {
            unsafe fn from_raw_handle(handle: std::os::windows::io::RawHandle) -> Self {
                Self {
                    $inner: unsafe { std::os::windows::io::FromRawHandle::from_raw_handle(handle) },
                }
            }
        }
        #[cfg(windows)]
        impl std::os::windows::io::AsHandle for $t {
            fn as_handle(&self) -> std::os::windows::io::BorrowedHandle {
                self.$inner.as_handle()
            }
        }
        #[cfg(windows)]
        impl std::os::windows::io::AsRawHandle for $t {
            fn as_raw_handle(&self) -> std::os::windows::io::RawHandle {
                self.$inner.as_raw_handle()
            }
        }
    };
    ($t:ty, $it:ty, $inner:ident,socket) => {
        $crate::impl_raw_fd!($t, $it, $inner);
        #[cfg(windows)]
        impl std::os::windows::io::FromRawSocket for $t {
            unsafe fn from_raw_socket(sock: std::os::windows::io::RawSocket) -> Self {
                Self {
                    $inner: unsafe { std::os::windows::io::FromRawSocket::from_raw_socket(sock) },
                }
            }
        }
        #[cfg(windows)]
        impl std::os::windows::io::AsSocket for $t {
            fn as_socket(&self) -> std::os::windows::io::BorrowedSocket {
                self.$inner.as_socket()
            }
        }
        #[cfg(windows)]
        impl std::os::windows::io::AsRawSocket for $t {
            fn as_raw_socket(&self) -> std::os::windows::io::RawSocket {
                self.$inner.as_raw_socket()
            }
        }
    };
}

/// The return type of [`Proactor::push`].
#[derive(Debug)]
pub enum PushEntry<K, R> {
    /// The operation is pushed to the submission queue.
    Pending(K),
    /// The operation is ready and returns.
    Ready(R),
}

impl<K, R> PushEntry<K, R> {
    /// Get if the current variant is [`PushEntry::Ready`].
    pub const fn is_ready(&self) -> bool {
        matches!(self, Self::Ready(_))
    }

    /// Take the ready variant if exists.
    pub fn take_ready(self) -> Option<R> {
        match self {
            Self::Pending(_) => None,
            Self::Ready(res) => Some(res),
        }
    }

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
        })
    }

    /// Get a default [`Extra`] for underlying driver.
    pub fn default_extra(&self) -> Extra {
        self.driver.default_extra().into()
    }

    /// The current driver type.
    pub fn driver_type(&self) -> DriverType {
        self.driver.driver_type()
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

    /// Cancel an operation with the pushed [`Key`].
    ///
    /// The cancellation is not reliable. The underlying operation may continue,
    /// but just don't return from [`Proactor::poll`].
    pub fn cancel<T: OpCode>(&mut self, key: Key<T>) -> Option<BufResult<usize, T>> {
        instrument!(compio_log::Level::DEBUG, "cancel", ?key);
        key.set_cancelled();
        if key.has_result() {
            Some(key.take_result())
        } else {
            self.driver.cancel(key);
            None
        }
    }

    /// Push an operation into the driver, and return the unique key [`Key`],
    /// associated with it.
    pub fn push<T: OpCode + 'static>(&mut self, op: T) -> PushEntry<Key<T>, BufResult<usize, T>> {
        let extra = self.default_extra();
        let key = Key::new(op, extra);
        match self.driver.push(key.clone().erase()) {
            Poll::Pending => PushEntry::Pending(key),
            Poll::Ready(res) => {
                key.set_result(res);
                PushEntry::Ready(key.take_result())
            }
        }
    }

    /// Poll the driver and get completed entries.
    /// You need to call [`Proactor::pop`] to get the pushed
    /// operations.
    pub fn poll(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.driver.poll(timeout)
    }

    /// Get the pushed operations from the completion entries.
    ///
    /// # Panics
    /// This function will panic if the requested operation has not been
    /// completed.
    pub fn pop<T>(&mut self, key: Key<T>) -> PushEntry<Key<T>, BufResult<usize, T>> {
        instrument!(compio_log::Level::DEBUG, "pop", ?key);
        if key.has_result() {
            PushEntry::Ready(key.take_result())
        } else {
            PushEntry::Pending(key)
        }
    }

    /// Get the pushed operations from the completion entries along the
    /// [`Extra`] associated.
    ///
    /// # Panics
    /// This function will panic if the requested operation has not been
    /// completed.
    pub fn pop_with_extra<T>(
        &mut self,
        key: Key<T>,
    ) -> PushEntry<Key<T>, (BufResult<usize, T>, Extra)> {
        instrument!(compio_log::Level::DEBUG, "pop", ?key);
        if key.has_result() {
            let extra = key.extra();
            let res = key.take_result();
            PushEntry::Ready((res, extra))
        } else {
            PushEntry::Pending(key)
        }
    }

    /// Update the waker of the specified op.
    pub fn update_waker<T>(&mut self, op: &mut Key<T>, waker: Waker) {
        op.set_waker(waker);
    }

    /// Create a waker to interrupt the inner driver.
    pub fn waker(&self) -> Waker {
        self.driver.waker()
    }

    /// Create buffer pool with given `buffer_size` and `buffer_len`
    ///
    /// # Notes
    ///
    /// If `buffer_len` is not a power of 2, it will be rounded up with
    /// [`u16::next_power_of_two`].
    pub fn create_buffer_pool(
        &mut self,
        buffer_len: u16,
        buffer_size: usize,
    ) -> io::Result<BufferPool> {
        self.driver.create_buffer_pool(buffer_len, buffer_size)
    }

    /// Release the buffer pool
    ///
    /// # Safety
    ///
    /// Caller must make sure to release the buffer pool with the correct
    /// driver, i.e., the one they created the buffer pool with.
    pub unsafe fn release_buffer_pool(&mut self, buffer_pool: BufferPool) -> io::Result<()> {
        unsafe { self.driver.release_buffer_pool(buffer_pool) }
    }
}

impl AsRawFd for Proactor {
    fn as_raw_fd(&self) -> RawFd {
        self.driver.as_raw_fd()
    }
}

/// An completed entry returned from kernel.
///
/// This represents the ownership of [`Key`] passed into the kernel is given
/// back from it to the driver.
#[derive(Debug)]
pub(crate) struct Entry {
    key: ErasedKey,
    result: io::Result<usize>,

    #[cfg(io_uring)]
    flags: u32,
}

unsafe impl Send for Entry {}
unsafe impl Sync for Entry {}

impl Entry {
    pub(crate) fn new(key: ErasedKey, result: io::Result<usize>) -> Self {
        #[cfg(not(io_uring))]
        {
            Self { key, result }
        }
        #[cfg(io_uring)]
        {
            Self {
                key,
                result,
                flags: 0,
            }
        }
    }

    #[allow(dead_code)]
    pub fn user_data(&self) -> usize {
        self.key.as_user_data()
    }

    #[allow(dead_code)]
    pub fn into_key(self) -> ErasedKey {
        self.key
    }

    #[cfg(io_uring)]
    pub fn flags(&self) -> u32 {
        self.flags
    }

    #[cfg(io_uring)]
    // this method only used by in io-uring driver
    pub(crate) fn set_flags(&mut self, flags: u32) {
        self.flags = flags;
    }

    pub fn notify(self) {
        #[cfg(io_uring)]
        self.key.borrow().extra_mut().set_flags(self.flags());
        self.key.set_result(self.result);
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
    sqpoll_idle: Option<Duration>,
    coop_taskrun: bool,
    taskrun_flag: bool,
    eventfd: Option<RawFd>,
    driver_type: Option<DriverType>,
}

// SAFETY: `RawFd` is thread safe.
unsafe impl Send for ProactorBuilder {}
unsafe impl Sync for ProactorBuilder {}

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
            sqpoll_idle: None,
            coop_taskrun: false,
            taskrun_flag: false,
            eventfd: None,
            driver_type: None,
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
    ///
    /// Warning: some operations don't work if the limit is set to zero:
    /// * `Asyncify` needs thread pool.
    /// * Operations except `Recv*`, `Send*`, `Connect`, `Accept` may need
    ///   thread pool.
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

    /// Set `io-uring` sqpoll idle milliseconds, when `sqpoll_idle` is set,
    /// io-uring sqpoll feature will be enabled
    ///
    /// # Notes
    ///
    /// - Only effective when the `io-uring` feature is enabled
    /// - `idle` must >= 1ms, otherwise will set sqpoll idle 0ms
    /// - `idle` will be rounded down
    pub fn sqpoll_idle(&mut self, idle: Duration) -> &mut Self {
        self.sqpoll_idle = Some(idle);
        self
    }

    /// `coop_taskrun` feature has been available since Linux Kernel 5.19. This
    /// will optimize performance for most cases, especially compio is a single
    /// thread runtime.
    ///
    /// However, it can't run with sqpoll feature.
    ///
    /// # Notes
    ///
    /// - Only effective when the `io-uring` feature is enabled
    pub fn coop_taskrun(&mut self, enable: bool) -> &mut Self {
        self.coop_taskrun = enable;
        self
    }

    /// `taskrun_flag` feature has been available since Linux Kernel 5.19. This
    /// allows io-uring driver can know if any cqes are available when try to
    /// push sqe to sq. This should be enabled with
    /// [`coop_taskrun`](Self::coop_taskrun)
    ///
    /// # Notes
    ///
    /// - Only effective when the `io-uring` feature is enabled
    pub fn taskrun_flag(&mut self, enable: bool) -> &mut Self {
        self.taskrun_flag = enable;
        self
    }

    /// Register an eventfd to io-uring.
    ///
    /// # Notes
    ///
    /// - Only effective when the `io-uring` feature is enabled
    pub fn register_eventfd(&mut self, fd: RawFd) -> &mut Self {
        self.eventfd = Some(fd);
        self
    }

    /// Force a driver type to use. It is ignored if the fusion driver is
    /// disabled.
    pub fn driver_type(&mut self, t: DriverType) -> &mut Self {
        self.driver_type = Some(t);
        self
    }

    /// Build the [`Proactor`].
    pub fn build(&self) -> io::Result<Proactor> {
        Proactor::with_builder(self)
    }
}
