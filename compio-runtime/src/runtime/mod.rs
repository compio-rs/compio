use std::{
    any::Any,
    cell::{Cell, RefCell},
    collections::HashSet,
    future::{Future, ready},
    io,
    panic::AssertUnwindSafe,
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
    time::Duration,
};

use async_task::Task;
use compio_buf::IntoInner;
use compio_driver::{
    AsRawFd, DriverType, Key, NotifyHandle, OpCode, Proactor, ProactorBuilder, PushEntry, RawFd,
    op::Asyncify,
};
use compio_log::{debug, instrument};
use futures_util::{FutureExt, future::Either};

pub(crate) mod op;
#[cfg(feature = "time")]
pub(crate) mod time;

mod buffer_pool;
pub use buffer_pool::*;

mod scheduler;

#[cfg(feature = "time")]
use crate::runtime::time::{TimerFuture, TimerKey, TimerRuntime};
use crate::{
    BufResult,
    affinity::bind_to_cpu_set,
    runtime::{op::OpFuture, scheduler::Scheduler},
};

scoped_tls::scoped_thread_local!(static CURRENT_RUNTIME: Runtime);

/// Type alias for `Task<Result<T, Box<dyn Any + Send>>>`, which resolves to an
/// `Err` when the spawned future panicked.
pub type JoinHandle<T> = Task<Result<T, Box<dyn Any + Send>>>;

thread_local! {
    static RUNTIME_ID: Cell<u64> = const { Cell::new(0) };
}

/// The async runtime of compio. It is a thread local runtime, and cannot be
/// sent to other threads.
pub struct Runtime {
    driver: RefCell<Proactor>,
    scheduler: Scheduler,
    #[cfg(feature = "time")]
    timer_runtime: RefCell<TimerRuntime>,
    // Runtime id is used to check if the buffer pool is belonged to this runtime or not.
    // Without this, if user enable `io-uring-buf-ring` feature then:
    // 1. Create a buffer pool at runtime1
    // 3. Create another runtime2, then use the exists buffer pool in runtime2, it may cause
    // - io-uring report error if the buffer group id is not registered
    // - buffer pool will return a wrong buffer which the buffer's data is uninit, that will cause
    //   UB
    id: u64,
}

impl Runtime {
    /// Create [`Runtime`] with default config.
    pub fn new() -> io::Result<Self> {
        Self::builder().build()
    }

    /// Create a builder for [`Runtime`].
    pub fn builder() -> RuntimeBuilder {
        RuntimeBuilder::new()
    }

    fn with_builder(builder: &RuntimeBuilder) -> io::Result<Self> {
        let RuntimeBuilder {
            proactor_builder,
            thread_affinity,
            event_interval,
        } = builder;
        let id = RUNTIME_ID.get();
        RUNTIME_ID.set(id + 1);
        if !thread_affinity.is_empty() {
            bind_to_cpu_set(thread_affinity);
        }
        Ok(Self {
            driver: RefCell::new(proactor_builder.build()?),
            scheduler: Scheduler::new(*event_interval),
            #[cfg(feature = "time")]
            timer_runtime: RefCell::new(TimerRuntime::new()),
            id,
        })
    }

    /// The current driver type.
    pub fn driver_type(&self) -> DriverType {
        self.driver.borrow().driver_type()
    }

    /// Try to perform a function on the current runtime, and if no runtime is
    /// running, return the function back.
    pub fn try_with_current<T, F: FnOnce(&Self) -> T>(f: F) -> Result<T, F> {
        if CURRENT_RUNTIME.is_set() {
            Ok(CURRENT_RUNTIME.with(f))
        } else {
            Err(f)
        }
    }

    /// Perform a function on the current runtime.
    ///
    /// ## Panics
    ///
    /// This method will panic if there are no running [`Runtime`].
    pub fn with_current<T, F: FnOnce(&Self) -> T>(f: F) -> T {
        #[cold]
        fn not_in_compio_runtime() -> ! {
            panic!("not in a compio runtime")
        }

        if CURRENT_RUNTIME.is_set() {
            CURRENT_RUNTIME.with(f)
        } else {
            not_in_compio_runtime()
        }
    }

    /// Set this runtime as current runtime, and perform a function in the
    /// current scope.
    pub fn enter<T, F: FnOnce() -> T>(&self, f: F) -> T {
        CURRENT_RUNTIME.set(self, f)
    }

    fn spawn_impl<F: Future + 'static>(&self, future: F) -> Task<F::Output> {
        let notify = self.driver.borrow().handle();
        self.scheduler.spawn(future, notify)
    }

    /// Low level API to control the runtime.
    ///
    /// Run the scheduled tasks.
    ///
    /// The return value indicates whether there are still tasks in the queue.
    pub fn run(&self) -> bool {
        self.scheduler.run()
    }

    /// Create a waker that notifies the runtime when woken.
    pub fn waker(&self) -> Waker {
        struct BlockOnWaker {
            notify: NotifyHandle,
        }

        impl Wake for BlockOnWaker {
            fn wake(self: Arc<Self>) {
                self.notify.notify().ok();
            }

            fn wake_by_ref(self: &Arc<Self>) {
                self.notify.notify().ok();
            }
        }

        let notify = self.driver.borrow().handle();
        Waker::from(Arc::new(BlockOnWaker { notify }))
    }

    /// Block on the future till it completes.
    pub fn block_on<F: Future>(&self, future: F) -> F::Output {
        self.enter(|| {
            let waker = self.waker();
            let mut context = Context::from_waker(&waker);
            let mut future = std::pin::pin!(future);
            loop {
                if let Poll::Ready(result) = future.as_mut().poll(&mut context) {
                    self.run();
                    return result;
                }
                let remaining_tasks = self.run();
                if remaining_tasks {
                    self.poll_with(Some(Duration::ZERO));
                } else {
                    self.poll();
                }
            }
        })
    }

    /// Spawns a new asynchronous task, returning a [`Task`] for it.
    ///
    /// Spawning a task enables the task to execute concurrently to other tasks.
    /// There is no guarantee that a spawned task will execute to completion.
    pub fn spawn<F: Future + 'static>(&self, future: F) -> JoinHandle<F::Output> {
        self.spawn_impl(AssertUnwindSafe(future).catch_unwind())
    }

    /// Spawns a blocking task in a new thread, and wait for it.
    ///
    /// The task will not be cancelled even if the future is dropped.
    pub fn spawn_blocking<T: Send + 'static>(
        &self,
        f: impl (FnOnce() -> T) + Send + 'static,
    ) -> JoinHandle<T> {
        let op = Asyncify::new(move || {
            let res = std::panic::catch_unwind(AssertUnwindSafe(f));
            BufResult(Ok(0), res)
        });
        // It is safe and sound to use `submit` here because the task is spawned
        // immediately.
        self.spawn_impl(self.submit(op).map(|res| res.1.into_inner()))
    }

    /// Attach a raw file descriptor/handle/socket to the runtime.
    ///
    /// You only need this when authoring your own high-level APIs. High-level
    /// resources in this crate are attached automatically.
    pub fn attach(&self, fd: RawFd) -> io::Result<()> {
        self.driver.borrow_mut().attach(fd)
    }

    fn submit_raw<T: OpCode + 'static>(&self, op: T) -> PushEntry<Key<T>, BufResult<usize, T>> {
        self.driver.borrow_mut().push(op)
    }

    /// Submit an operation to the runtime.
    ///
    /// You only need this when authoring your own [`OpCode`].
    ///
    /// It is safe to send the returned future to another runtime and poll it,
    /// but the exact behavior is not guaranteed, e.g. it may return pending
    /// forever or else.
    fn submit<T: OpCode + 'static>(&self, op: T) -> impl Future<Output = BufResult<usize, T>> {
        self.submit_with_flags(op).map(|(res, _)| res)
    }

    /// Submit an operation to the runtime.
    ///
    /// The difference between [`Runtime::submit`] is this method will return
    /// the flags
    ///
    /// You only need this when authoring your own [`OpCode`].
    ///
    /// It is safe to send the returned future to another runtime and poll it,
    /// but the exact behavior is not guaranteed, e.g. it may return pending
    /// forever or else.
    fn submit_with_flags<T: OpCode + 'static>(
        &self,
        op: T,
    ) -> impl Future<Output = (BufResult<usize, T>, u32)> {
        match self.submit_raw(op) {
            PushEntry::Pending(user_data) => Either::Left(OpFuture::new(user_data)),
            PushEntry::Ready(res) => {
                // submit_flags won't be ready immediately, if ready, it must be error without
                // flags
                Either::Right(ready((res, 0)))
            }
        }
    }

    pub(crate) fn cancel_op<T: OpCode>(&self, op: Key<T>) {
        self.driver.borrow_mut().cancel(op);
    }

    #[cfg(feature = "time")]
    pub(crate) fn cancel_timer(&self, key: &TimerKey) {
        self.timer_runtime.borrow_mut().cancel(key);
    }

    pub(crate) fn poll_task<T: OpCode>(
        &self,
        cx: &mut Context,
        op: Key<T>,
    ) -> PushEntry<Key<T>, (BufResult<usize, T>, u32)> {
        instrument!(compio_log::Level::DEBUG, "poll_task", ?op);
        let mut driver = self.driver.borrow_mut();
        driver.pop(op).map_pending(|mut k| {
            driver.update_waker(&mut k, cx.waker().clone());
            k
        })
    }

    #[cfg(feature = "time")]
    pub(crate) fn poll_timer(&self, cx: &mut Context, key: &TimerKey) -> Poll<()> {
        instrument!(compio_log::Level::DEBUG, "poll_timer", ?cx, ?key);
        let mut timer_runtime = self.timer_runtime.borrow_mut();
        if timer_runtime.remove_completed(key) {
            debug!("ready");
            Poll::Ready(())
        } else {
            debug!("pending");
            timer_runtime.update_waker(key, cx.waker().clone());
            Poll::Pending
        }
    }

    /// Low level API to control the runtime.
    ///
    /// Get the timeout value to be passed to [`Proactor::poll`].
    pub fn current_timeout(&self) -> Option<Duration> {
        #[cfg(not(feature = "time"))]
        let timeout = None;
        #[cfg(feature = "time")]
        let timeout = self.timer_runtime.borrow().min_timeout();
        timeout
    }

    /// Low level API to control the runtime.
    ///
    /// Poll the inner proactor. It is equal to calling [`Runtime::poll_with`]
    /// with [`Runtime::current_timeout`].
    pub fn poll(&self) {
        instrument!(compio_log::Level::DEBUG, "poll");
        let timeout = self.current_timeout();
        debug!("timeout: {:?}", timeout);
        self.poll_with(timeout)
    }

    /// Low level API to control the runtime.
    ///
    /// Poll the inner proactor with a custom timeout.
    pub fn poll_with(&self, timeout: Option<Duration>) {
        instrument!(compio_log::Level::DEBUG, "poll_with");

        let mut driver = self.driver.borrow_mut();
        match driver.poll(timeout) {
            Ok(()) => {}
            Err(e) => match e.kind() {
                io::ErrorKind::TimedOut | io::ErrorKind::Interrupted => {
                    debug!("expected error: {e}");
                }
                _ => panic!("{e:?}"),
            },
        }
        #[cfg(feature = "time")]
        self.timer_runtime.borrow_mut().wake();
    }

    pub(crate) fn create_buffer_pool(
        &self,
        buffer_len: u16,
        buffer_size: usize,
    ) -> io::Result<compio_driver::BufferPool> {
        self.driver
            .borrow_mut()
            .create_buffer_pool(buffer_len, buffer_size)
    }

    pub(crate) unsafe fn release_buffer_pool(
        &self,
        buffer_pool: compio_driver::BufferPool,
    ) -> io::Result<()> {
        self.driver.borrow_mut().release_buffer_pool(buffer_pool)
    }

    pub(crate) fn id(&self) -> u64 {
        self.id
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.enter(|| {
            self.scheduler.clear();
        })
    }
}

impl AsRawFd for Runtime {
    fn as_raw_fd(&self) -> RawFd {
        self.driver.borrow().as_raw_fd()
    }
}

#[cfg(feature = "criterion")]
impl criterion::async_executor::AsyncExecutor for Runtime {
    fn block_on<T>(&self, future: impl Future<Output = T>) -> T {
        self.block_on(future)
    }
}

#[cfg(feature = "criterion")]
impl criterion::async_executor::AsyncExecutor for &Runtime {
    fn block_on<T>(&self, future: impl Future<Output = T>) -> T {
        (**self).block_on(future)
    }
}

/// Builder for [`Runtime`].
#[derive(Debug, Clone)]
pub struct RuntimeBuilder {
    proactor_builder: ProactorBuilder,
    thread_affinity: HashSet<usize>,
    event_interval: usize,
}

impl Default for RuntimeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeBuilder {
    /// Create the builder with default config.
    pub fn new() -> Self {
        Self {
            proactor_builder: ProactorBuilder::new(),
            event_interval: 61,
            thread_affinity: HashSet::new(),
        }
    }

    /// Replace proactor builder.
    pub fn with_proactor(&mut self, builder: ProactorBuilder) -> &mut Self {
        self.proactor_builder = builder;
        self
    }

    /// Sets the thread affinity for the runtime.
    pub fn thread_affinity(&mut self, cpus: HashSet<usize>) -> &mut Self {
        self.thread_affinity = cpus;
        self
    }

    /// Sets the number of scheduler ticks after which the scheduler will poll
    /// for external events (timers, I/O, and so on).
    ///
    /// A scheduler “tick” roughly corresponds to one poll invocation on a task.
    pub fn event_interval(&mut self, val: usize) -> &mut Self {
        self.event_interval = val;
        self
    }

    /// Build [`Runtime`].
    pub fn build(&self) -> io::Result<Runtime> {
        Runtime::with_builder(self)
    }
}

/// Spawns a new asynchronous task, returning a [`Task`] for it.
///
/// Spawning a task enables the task to execute concurrently to other tasks.
/// There is no guarantee that a spawned task will execute to completion.
///
/// ```
/// # compio_runtime::Runtime::new().unwrap().block_on(async {
/// let task = compio_runtime::spawn(async {
///     println!("Hello from a spawned task!");
///     42
/// });
///
/// assert_eq!(
///     task.await.unwrap_or_else(|e| std::panic::resume_unwind(e)),
///     42
/// );
/// # })
/// ```
///
/// ## Panics
///
/// This method doesn't create runtime. It tries to obtain the current runtime
/// by [`Runtime::with_current`].
pub fn spawn<F: Future + 'static>(future: F) -> JoinHandle<F::Output> {
    Runtime::with_current(|r| r.spawn(future))
}

/// Spawns a blocking task in a new thread, and wait for it.
///
/// The task will not be cancelled even if the future is dropped.
///
/// ## Panics
///
/// This method doesn't create runtime. It tries to obtain the current runtime
/// by [`Runtime::with_current`].
pub fn spawn_blocking<T: Send + 'static>(
    f: impl (FnOnce() -> T) + Send + 'static,
) -> JoinHandle<T> {
    Runtime::with_current(|r| r.spawn_blocking(f))
}

/// Submit an operation to the current runtime, and return a future for it.
///
/// ## Panics
///
/// This method doesn't create runtime. It tries to obtain the current runtime
/// by [`Runtime::with_current`].
pub async fn submit<T: OpCode + 'static>(op: T) -> BufResult<usize, T> {
    submit_with_flags(op).await.0
}

/// Submit an operation to the current runtime, and return a future for it with
/// flags.
///
/// ## Panics
///
/// This method doesn't create runtime. It tries to obtain the current runtime
/// by [`Runtime::with_current`].
pub async fn submit_with_flags<T: OpCode + 'static>(op: T) -> (BufResult<usize, T>, u32) {
    Runtime::with_current(|r| r.submit_with_flags(op)).await
}

#[cfg(feature = "time")]
pub(crate) async fn create_timer(instant: std::time::Instant) {
    let key = Runtime::with_current(|r| r.timer_runtime.borrow_mut().insert(instant));
    if let Some(key) = key {
        TimerFuture::new(key).await
    }
}
