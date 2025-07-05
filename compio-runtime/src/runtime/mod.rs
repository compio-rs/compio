use std::{
    any::Any,
    cell::{Cell, RefCell},
    collections::VecDeque,
    future::{Future, ready},
    io,
    marker::PhantomData,
    panic::AssertUnwindSafe,
    rc::Rc,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use async_task::{Runnable, Task};
use compio_buf::IntoInner;
use compio_driver::{
    AsRawFd, Key, NotifyHandle, OpCode, Proactor, ProactorBuilder, PushEntry, RawFd, op::Asyncify,
};
use compio_log::{debug, instrument};
use crossbeam_queue::SegQueue;
use futures_util::{FutureExt, future::Either};

pub(crate) mod op;
#[cfg(feature = "time")]
pub(crate) mod time;

mod buffer_pool;
pub use buffer_pool::*;

mod send_wrapper;
use send_wrapper::SendWrapper;

#[cfg(feature = "time")]
use crate::runtime::time::{TimerFuture, TimerRuntime};
use crate::{BufResult, affinity::bind_to_cpu_set, runtime::op::OpFuture};

scoped_tls::scoped_thread_local!(static CURRENT_RUNTIME: Runtime);

/// Type alias for `Task<Result<T, Box<dyn Any + Send>>>`, which resolves to an
/// `Err` when the spawned future panicked.
pub type JoinHandle<T> = Task<Result<T, Box<dyn Any + Send>>>;

struct RunnableQueue {
    local_runnables: SendWrapper<RefCell<VecDeque<Runnable>>>,
    sync_runnables: SegQueue<Runnable>,
}

impl RunnableQueue {
    pub fn new() -> Self {
        Self {
            local_runnables: SendWrapper::new(RefCell::new(VecDeque::new())),
            sync_runnables: SegQueue::new(),
        }
    }

    pub fn schedule(&self, runnable: Runnable, handle: &NotifyHandle) {
        if let Some(runnables) = self.local_runnables.get() {
            runnables.borrow_mut().push_back(runnable);
            #[cfg(feature = "notify-always")]
            handle.notify().ok();
        } else {
            self.sync_runnables.push(runnable);
            handle.notify().ok();
        }
    }

    /// SAFETY: call in the main thread
    pub unsafe fn run(&self, event_interval: usize) -> bool {
        let local_runnables = self.local_runnables.get_unchecked();
        for _i in 0..event_interval {
            let next_task = local_runnables.borrow_mut().pop_front();
            let has_local_task = next_task.is_some();
            if let Some(task) = next_task {
                task.run();
            }
            // Cheaper than pop.
            let has_sync_task = !self.sync_runnables.is_empty();
            if has_sync_task {
                if let Some(task) = self.sync_runnables.pop() {
                    task.run();
                }
            } else if !has_local_task {
                break;
            }
        }
        !(local_runnables.borrow_mut().is_empty() && self.sync_runnables.is_empty())
    }
}

thread_local! {
    static RUNTIME_ID: Cell<u64> = const { Cell::new(0) };
}

/// The async runtime of compio. It is a thread local runtime, and cannot be
/// sent to other threads.
pub struct Runtime {
    driver: RefCell<Proactor>,
    runnables: Arc<RunnableQueue>,
    #[cfg(feature = "time")]
    timer_runtime: RefCell<TimerRuntime>,
    event_interval: usize,
    // Runtime id is used to check if the buffer pool is belonged to this runtime or not.
    // Without this, if user enable `io-uring-buf-ring` feature then:
    // 1. Create a buffer pool at runtime1
    // 3. Create another runtime2, then use the exists buffer pool in runtime2, it may cause
    // - io-uring report error if the buffer group id is not registered
    // - buffer pool will return a wrong buffer which the buffer's data is uninit, that will cause
    //   UB
    id: u64,
    // Other fields don't make it !Send, but actually `local_runnables` implies it should be !Send,
    // otherwise it won't be valid if the runtime is sent to other threads.
    _p: PhantomData<Rc<VecDeque<Runnable>>>,
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

    fn with_builder(builder: RuntimeBuilder) -> io::Result<Self> {
        let RuntimeBuilder {
            proactor_builder,
            thread_affinity,
            event_interval,
        } = builder;
        let id = RUNTIME_ID.get();
        RUNTIME_ID.set(id + 1);
        if let Some(cpus) = thread_affinity {
            bind_to_cpu_set(cpus)?;
        }
        Ok(Self {
            driver: RefCell::new(proactor_builder.build()?),
            runnables: Arc::new(RunnableQueue::new()),
            #[cfg(feature = "time")]
            timer_runtime: RefCell::new(TimerRuntime::new()),
            event_interval,
            id,
            _p: PhantomData,
        })
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

    /// Spawns a new asynchronous task, returning a [`Task`] for it.
    ///
    /// # Safety
    ///
    /// The caller should ensure the captured lifetime long enough.
    pub unsafe fn spawn_unchecked<F: Future>(&self, future: F) -> Task<F::Output> {
        let runnables = self.runnables.clone();
        let handle = self.driver.borrow().handle();
        let schedule = move |runnable| {
            runnables.schedule(runnable, &handle);
        };
        let (runnable, task) = async_task::spawn_unchecked(future, schedule);
        runnable.schedule();
        task
    }

    /// Low level API to control the runtime.
    ///
    /// Run the scheduled tasks.
    ///
    /// The return value indicates whether there are still tasks in the queue.
    pub fn run(&self) -> bool {
        // SAFETY: self is !Send + !Sync.
        unsafe { self.runnables.run(self.event_interval) }
    }

    /// Block on the future till it completes.
    pub fn block_on<F: Future>(&self, future: F) -> F::Output {
        CURRENT_RUNTIME.set(self, || {
            let mut result = None;
            unsafe { self.spawn_unchecked(async { result = Some(future.await) }) }.detach();
            loop {
                let remaining_tasks = self.run();
                if let Some(result) = result.take() {
                    return result;
                }
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
        unsafe { self.spawn_unchecked(AssertUnwindSafe(future).catch_unwind()) }
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
        #[allow(deprecated)]
        unsafe {
            self.spawn_unchecked(self.submit(op).map(|res| res.1.into_inner()))
        }
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
    #[deprecated = "use compio::runtime::submit instead"]
    pub fn submit<T: OpCode + 'static>(&self, op: T) -> impl Future<Output = BufResult<usize, T>> {
        #[allow(deprecated)]
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
    #[deprecated = "use compio::runtime::submit_with_flags instead"]
    pub fn submit_with_flags<T: OpCode + 'static>(
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
    pub(crate) fn cancel_timer(&self, key: usize) {
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
    pub(crate) fn poll_timer(&self, cx: &mut Context, key: usize) -> Poll<()> {
        instrument!(compio_log::Level::DEBUG, "poll_timer", ?cx, ?key);
        let mut timer_runtime = self.timer_runtime.borrow_mut();
        if !timer_runtime.is_completed(key) {
            debug!("pending");
            timer_runtime.update_waker(key, cx.waker().clone());
            Poll::Pending
        } else {
            debug!("ready");
            Poll::Ready(())
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
            while self.runnables.sync_runnables.pop().is_some() {}
            let local_runnables = unsafe { self.runnables.local_runnables.get_unchecked() };
            loop {
                let runnable = local_runnables.borrow_mut().pop_front();
                if runnable.is_none() {
                    break;
                }
            }
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
    thread_affinity: Option<Vec<usize>>,
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
            thread_affinity: None,
        }
    }

    /// Replace proactor builder.
    pub fn with_proactor(mut self, builder: ProactorBuilder) -> Self {
        self.proactor_builder = builder;
        self
    }

    /// Sets the thread affinity for the runtime.
    pub fn thread_affinity(mut self, cpus: Vec<usize>) -> Self {
        self.thread_affinity = Some(cpus);
        self
    }

    /// Sets the number of scheduler ticks after which the scheduler will poll
    /// for external events (timers, I/O, and so on).
    ///
    /// A scheduler “tick” roughly corresponds to one poll invocation on a task.
    pub fn event_interval(mut self, val: usize) -> Self {
        self.event_interval = val;
        self
    }

    /// Build [`Runtime`].
    pub fn build(self) -> io::Result<Runtime> {
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
    let state = Runtime::with_current(|r| r.submit_raw(op));
    match state {
        PushEntry::Pending(user_data) => OpFuture::new(user_data).await,
        PushEntry::Ready(res) => {
            // submit_flags won't be ready immediately, if ready, it must be error without
            // flags, or the flags are not necessary
            (res, 0)
        }
    }
}

#[cfg(feature = "time")]
pub(crate) async fn create_timer(instant: std::time::Instant) {
    let key = Runtime::with_current(|r| r.timer_runtime.borrow_mut().insert(instant));
    if let Some(key) = key {
        TimerFuture::new(key).await
    }
}
