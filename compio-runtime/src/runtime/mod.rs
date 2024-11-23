use std::{
    any::Any,
    cell::RefCell,
    collections::VecDeque,
    future::{Future, poll_fn, ready},
    io,
    marker::PhantomData,
    panic::AssertUnwindSafe,
    rc::Rc,
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

mod send_wrapper;
use send_wrapper::SendWrapper;

#[cfg(feature = "time")]
use crate::runtime::time::{TimerFuture, TimerRuntime};
use crate::{BufResult, runtime::op::OpFuture};

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

/// The async runtime of compio. It is a thread local runtime, and cannot be
/// sent to other threads.
pub struct Runtime {
    // The runnable queue should live longer than the proactor.
    runnables: Box<RunnableQueue>,
    driver: RefCell<Proactor>,
    #[cfg(feature = "time")]
    timer_runtime: RefCell<TimerRuntime>,
    event_interval: usize,
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

    fn with_builder(builder: &RuntimeBuilder) -> io::Result<Self> {
        Ok(Self {
            driver: RefCell::new(builder.proactor_builder.build()?),
            runnables: Box::new(RunnableQueue::new()),
            #[cfg(feature = "time")]
            timer_runtime: RefCell::new(TimerRuntime::new()),
            event_interval: builder.event_interval,
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
        let runnables = self.runnables.as_ref() as *const RunnableQueue;
        let handle = self
            .driver
            .borrow()
            .handle()
            .expect("cannot create notify handle of the proactor");
        let schedule = move |runnable| {
            // The schedule closure are owned by runnables, and the runnables are owned by
            // the queue. This is a self-reference.
            let runnables = &*runnables;
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
        f: impl (FnOnce() -> T) + Send + Sync + 'static,
    ) -> JoinHandle<T> {
        let op = Asyncify::new(move || {
            let res = std::panic::catch_unwind(AssertUnwindSafe(f));
            BufResult(Ok(0), res)
        });
        let closure = async move {
            let mut op = op;
            loop {
                match self.submit(op).await {
                    BufResult(Ok(_), rop) => break rop.into_inner(),
                    BufResult(Err(_), rop) => op = rop,
                }
                // Possible error: thread pool is full, or failed to create notify handle.
                // Push the future to the back of the queue.
                let mut yielded = false;
                poll_fn(|cx| {
                    if yielded {
                        Poll::Ready(())
                    } else {
                        yielded = true;
                        cx.waker().wake_by_ref();
                        Poll::Pending
                    }
                })
                .await;
            }
        };
        // SAFETY: the closure catches the shared reference of self, which is in an Rc
        // so it won't be moved.
        unsafe { self.spawn_unchecked(closure) }
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
    pub fn submit<T: OpCode + 'static>(&self, op: T) -> impl Future<Output = BufResult<usize, T>> {
        self.submit_with_flags(op).map(|(res, _)| res)
    }

    /// Submit an operation to the runtime.
    ///
    /// The difference between [`Runtime::submit`] is this method will return
    /// the flags
    ///
    /// You only need this when authoring your own [`OpCode`].
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

    #[cfg(feature = "time")]
    pub(crate) fn create_timer(&self, delay: std::time::Duration) -> impl Future<Output = ()> {
        let mut timer_runtime = self.timer_runtime.borrow_mut();
        if let Some(key) = timer_runtime.insert(delay) {
            Either::Left(TimerFuture::new(key))
        } else {
            Either::Right(std::future::ready(()))
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
        }
    }

    /// Replace proactor builder.
    pub fn with_proactor(&mut self, builder: ProactorBuilder) -> &mut Self {
        self.proactor_builder = builder;
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
    f: impl (FnOnce() -> T) + Send + Sync + 'static,
) -> JoinHandle<T> {
    Runtime::with_current(|r| r.spawn_blocking(f))
}

/// Submit an operation to the current runtime, and return a future for it.
///
/// ## Panics
///
/// This method doesn't create runtime. It tries to obtain the current runtime
/// by [`Runtime::with_current`].
pub fn submit<T: OpCode + 'static>(op: T) -> impl Future<Output = BufResult<usize, T>> {
    Runtime::with_current(|r| r.submit(op))
}

/// Submit an operation to the current runtime, and return a future for it with
/// flags.
///
/// ## Panics
///
/// This method doesn't create runtime. It tries to obtain the current runtime
/// by [`Runtime::with_current`].
pub fn submit_with_flags<T: OpCode + 'static>(
    op: T,
) -> impl Future<Output = (BufResult<usize, T>, u32)> {
    Runtime::with_current(|r| r.submit_with_flags(op))
}
