//! Platform-specific drivers.
//!
//! Some types differ by compilation target.

#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(feature = "once_cell_try", feature(once_cell_try))]
#![warn(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]
#![doc(
    html_logo_url = "https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-bold.svg"
)]
#![doc(
    html_favicon_url = "https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-bold.svg"
)]

use std::{
    io,
    task::{Poll, Waker},
    time::Duration,
};

use compio_buf::BufResult;
use compio_log::instrument;

mod macros;

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

mod cancel;
pub use cancel::*;

use crate::key::ErasedKey;

mod sys_slice;

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
    cancel: CancelRegistry,
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
            cancel: CancelRegistry::new(),
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
    /// Returns the result if the key is unique and the operation is completed.
    ///
    /// The cancellation is not reliable. The underlying operation may continue,
    /// but just don't return from [`Proactor::poll`].
    pub fn cancel<T: OpCode>(&mut self, key: Key<T>) -> Option<BufResult<usize, T>> {
        instrument!(compio_log::Level::DEBUG, "cancel", ?key);
        if key.set_cancelled() {
            return None;
        }
        self.cancel.remove(&key);
        if key.is_unique() && key.has_result() {
            Some(key.take_result())
        } else {
            self.driver.cancel(key.erase());
            None
        }
    }

    /// Cancel an operation with a [`Cancel`] token.
    ///
    /// Returns if a cancellation has been issued.
    ///
    /// The cancellation is not reliable. The underlying operation may continue,
    /// but just don't return from [`Proactor::pop`]. This will do nothing if
    /// the operation has already been completed or cancelled before.
    pub fn cancel_token(&mut self, token: Cancel) -> bool {
        let Some(key) = self.cancel.take(token) else {
            return false;
        };
        if key.set_cancelled() || key.has_result() {
            return false;
        }
        self.driver.cancel(key);
        true
    }

    /// Create a [`Cancel`] that can be used to cancel the operation even
    /// without the key.
    ///
    /// This acts like a weak reference to the [`Key`], but can only be used to
    /// cancel the operation with [`Proactor::cancel_token`]. Extra copy of
    /// [`Key`] may cause [`Proactor::pop`] to panic while keys registered
    /// as [`Cancel`] will be properly handled. So this is useful in cases
    /// where you're not sure if the operation will be cancelled.
    pub fn register_cancel<T: OpCode>(&mut self, key: &Key<T>) -> Cancel {
        self.cancel.register(key)
    }

    /// Push an operation into the driver, and return the unique key [`Key`],
    /// associated with it.
    pub fn push<T: sys::OpCode + 'static>(
        &mut self,
        op: T,
    ) -> PushEntry<Key<T>, BufResult<usize, T>> {
        self.push_with_extra(op, self.default_extra())
    }

    /// Push an operation into the driver with user-defined [`Extra`], and
    /// return the unique key [`Key`], associated with it.
    pub fn push_with_extra<T: sys::OpCode + 'static>(
        &mut self,
        op: T,
        extra: Extra,
    ) -> PushEntry<Key<T>, BufResult<usize, T>> {
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
    ///
    /// This function will panic if the [`Key`] is not unique.
    pub fn pop<T>(&mut self, key: Key<T>) -> PushEntry<Key<T>, BufResult<usize, T>> {
        instrument!(compio_log::Level::DEBUG, "pop", ?key);
        if key.has_result() {
            self.cancel.remove(&key);
            PushEntry::Ready(key.take_result())
        } else {
            PushEntry::Pending(key)
        }
    }

    /// Get the pushed operations from the completion entries along the
    /// [`Extra`] associated.
    ///
    /// # Panics
    ///
    /// This function will panic if the [`Key`] is not unique.
    pub fn pop_with_extra<T>(
        &mut self,
        key: Key<T>,
    ) -> PushEntry<Key<T>, (BufResult<usize, T>, Extra)> {
        instrument!(compio_log::Level::DEBUG, "pop", ?key);
        if key.has_result() {
            self.cancel.remove(&key);
            let extra = key.swap_extra(self.default_extra());
            let res = key.take_result();
            PushEntry::Ready((res, extra))
        } else {
            PushEntry::Pending(key)
        }
    }

    /// Update the waker of the specified op.
    pub fn update_waker<T>(&mut self, op: &mut Key<T>, waker: &Waker) {
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

    /// Register a new personality in io-uring driver.
    ///
    /// Returns the personality id, which can be used with
    /// [`Extra::set_personality`] to set the personality for an operation.
    ///
    /// This only works on `io_uring` driver. It will return an [`Unsupported`]
    /// error on other drivers. See [`Submitter::register_personality`] for
    /// more.
    ///
    /// [`Unsupported`]: std::io::ErrorKind::Unsupported
    /// [`Submitter::register_personality`]: https://docs.rs/io-uring/latest/io_uring/struct.Submitter.html#method.register_personality
    pub fn register_personality(&self) -> io::Result<u16> {
        fn unsupported() -> io::Error {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "Personality is only supported on io-uring driver",
            )
        }

        #[cfg(io_uring)]
        match self.driver.as_iour() {
            Some(iour) => iour.register_personality(),
            None => Err(unsupported()),
        }

        #[cfg(not(io_uring))]
        Err(unsupported())
    }

    /// Unregister the given personality in io-uring driver.
    ///
    /// This only works on `io_uring` driver. It will return an [`Unsupported`]
    /// error on other drivers. See [`Submitter::unregister_personality`] for
    /// more.
    ///
    /// [`Unsupported`]: std::io::ErrorKind::Unsupported
    /// [`Submitter::unregister_personality`]: https://docs.rs/io-uring/latest/io_uring/struct.Submitter.html#method.unregister_personality
    pub fn unregister_personality(&self, personality: u16) -> io::Result<()> {
        fn unsupported(_: u16) -> io::Error {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "Personality is only supported on io-uring driver",
            )
        }

        #[cfg(io_uring)]
        match self.driver.as_iour() {
            Some(iour) => iour.unregister_personality(personality),
            None => Err(unsupported(personality)),
        }

        #[cfg(not(io_uring))]
        Err(unsupported(personality))
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
        self.key.as_raw()
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

    /// Set `io-uring` sqpoll idle duration,
    ///
    /// This will also enable io-uring's sqpoll feature.
    ///
    /// # Notes
    ///
    /// - Only effective when the `io-uring` feature is enabled
    /// - `idle` must be >= 1ms, otherwise sqpoll idle will be set to 0 ms
    /// - `idle` will be rounded down
    pub fn sqpoll_idle(&mut self, idle: Duration) -> &mut Self {
        self.sqpoll_idle = Some(idle);
        self
    }

    /// Optimize performance for most cases, especially compio is a single
    /// thread runtime.
    ///
    /// However, it can't run with sqpoll feature.
    ///
    /// # Notes
    ///
    /// - Available since Linux Kernel 5.19.
    /// - Only effective when the `io-uring` feature is enabled
    pub fn coop_taskrun(&mut self, enable: bool) -> &mut Self {
        self.coop_taskrun = enable;
        self
    }

    /// Allows io-uring driver to know if any cqe's are available when try to
    /// push an sqe to the submission queue.
    ///
    /// This should be enabled with [`coop_taskrun`](Self::coop_taskrun)
    ///
    /// # Notes
    ///
    /// - Available since Linux Kernel 5.19.
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

    /// Force a driver type to use.
    ///
    /// It is ignored if the fusion driver is disabled.
    pub fn driver_type(&mut self, t: DriverType) -> &mut Self {
        self.driver_type = Some(t);
        self
    }

    /// Build the [`Proactor`].
    pub fn build(&self) -> io::Result<Proactor> {
        Proactor::with_builder(self)
    }
}
