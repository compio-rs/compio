use std::{
    any::Any,
    cell::RefCell,
    collections::VecDeque,
    future::{poll_fn, ready, Future},
    io,
    panic::AssertUnwindSafe,
    rc::{Rc, Weak},
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use async_task::{Runnable, Task};
use compio_buf::IntoInner;
use compio_driver::{
    op::Asyncify, AsRawFd, Key, OpCode, Proactor, ProactorBuilder, PushEntry, RawFd,
};
use compio_log::{debug, instrument};
use crossbeam_queue::SegQueue;
use futures_util::{future::Either, FutureExt};
use send_wrapper::SendWrapper;
use smallvec::SmallVec;

pub(crate) mod op;
#[cfg(feature = "time")]
pub(crate) mod time;

#[cfg(feature = "time")]
use crate::runtime::time::{TimerFuture, TimerRuntime};
use crate::{runtime::op::OpFuture, BufResult};

pub type JoinHandle<T> = Task<Result<T, Box<dyn Any + Send>>>;

pub(crate) struct RuntimeInner {
    driver: RefCell<Proactor>,
    local_runnables: Arc<SendWrapper<RefCell<VecDeque<Runnable>>>>,
    sync_runnables: Arc<SegQueue<Runnable>>,
    #[cfg(feature = "time")]
    timer_runtime: RefCell<TimerRuntime>,
}

impl RuntimeInner {
    pub fn new(builder: &ProactorBuilder) -> io::Result<Self> {
        Ok(Self {
            driver: RefCell::new(builder.build()?),
            local_runnables: Arc::new(SendWrapper::new(RefCell::new(VecDeque::new()))),
            sync_runnables: Arc::new(SegQueue::new()),
            #[cfg(feature = "time")]
            timer_runtime: RefCell::new(TimerRuntime::new()),
        })
    }

    // Safety: be careful about the captured lifetime.
    pub unsafe fn spawn_unchecked<F: Future>(&self, future: F) -> Task<F::Output> {
        let local_runnables = self.local_runnables.clone();
        let sync_runnables = self.sync_runnables.clone();
        let handle = self
            .driver
            .borrow()
            .handle()
            .expect("cannot create notify handle of the proactor");
        let schedule = move |runnable| {
            if local_runnables.valid() {
                local_runnables.borrow_mut().push_back(runnable);
            } else {
                sync_runnables.push(runnable);
                handle.notify().ok();
            }
        };
        let (runnable, task) = async_task::spawn_unchecked(future, schedule);
        runnable.schedule();
        task
    }

    pub fn run(&self) {
        use std::ops::Deref;

        let local_runnables = self.local_runnables.deref().deref();
        loop {
            let next_task = local_runnables.borrow_mut().pop_front();
            let has_local_task = next_task.is_some();
            if let Some(task) = next_task {
                task.run();
            }
            let next_task = self.sync_runnables.pop();
            let has_sync_task = next_task.is_some();
            if let Some(task) = next_task {
                task.run();
            }
            if !has_local_task && !has_sync_task {
                break;
            }
        }
    }

    pub fn block_on<F: Future>(&self, future: F) -> F::Output {
        let mut result = None;
        unsafe { self.spawn_unchecked(async { result = Some(future.await) }) }.detach();
        loop {
            self.run();
            if let Some(result) = result.take() {
                return result;
            }
            self.poll();
        }
    }

    pub fn spawn<F: Future + 'static>(&self, future: F) -> JoinHandle<F::Output> {
        unsafe { self.spawn_unchecked(AssertUnwindSafe(future).catch_unwind()) }
    }

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

    pub fn attach(&self, fd: RawFd) -> io::Result<()> {
        self.driver.borrow_mut().attach(fd)
    }

    pub fn submit_raw<T: OpCode + 'static>(&self, op: T) -> PushEntry<Key<T>, BufResult<usize, T>> {
        self.driver.borrow_mut().push(op)
    }

    pub fn submit<T: OpCode + 'static>(&self, op: T) -> impl Future<Output = BufResult<usize, T>> {
        match self.submit_raw(op) {
            PushEntry::Pending(user_data) => Either::Left(OpFuture::new(user_data)),
            PushEntry::Ready(res) => Either::Right(ready(res)),
        }
    }

    #[cfg(feature = "time")]
    pub fn create_timer(&self, delay: std::time::Duration) -> impl Future<Output = ()> {
        let mut timer_runtime = self.timer_runtime.borrow_mut();
        if let Some(key) = timer_runtime.insert(delay) {
            Either::Left(TimerFuture::new(key))
        } else {
            Either::Right(std::future::ready(()))
        }
    }

    pub fn cancel_op<T: OpCode>(&self, op: Key<T>) {
        self.driver.borrow_mut().cancel(op);
    }

    #[cfg(feature = "time")]
    pub fn cancel_timer(&self, key: usize) {
        self.timer_runtime.borrow_mut().cancel(key);
    }

    pub fn poll_task<T: OpCode>(
        &self,
        cx: &mut Context,
        op: Key<T>,
    ) -> PushEntry<Key<T>, BufResult<usize, T>> {
        instrument!(compio_log::Level::DEBUG, "poll_task", ?op);
        let mut driver = self.driver.borrow_mut();
        driver.pop(op).map_pending(|mut k| {
            driver.update_waker(&mut k, cx.waker().clone());
            k
        })
    }

    #[cfg(feature = "time")]
    pub fn poll_timer(&self, cx: &mut Context, key: usize) -> Poll<()> {
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

    pub fn current_timeout(&self) -> Option<Duration> {
        #[cfg(not(feature = "time"))]
        let timeout = None;
        #[cfg(feature = "time")]
        let timeout = self.timer_runtime.borrow().min_timeout();
        timeout
    }

    pub fn poll(&self) {
        instrument!(compio_log::Level::DEBUG, "poll");
        let timeout = self.current_timeout();
        debug!("timeout: {:?}", timeout);
        self.poll_with(timeout)
    }

    pub fn poll_with(&self, timeout: Option<Duration>) {
        instrument!(compio_log::Level::DEBUG, "poll_with");

        let mut entries = SmallVec::<[usize; 1024]>::new();
        let mut driver = self.driver.borrow_mut();
        match driver.poll(timeout, &mut entries) {
            Ok(_) => {
                debug!("poll driver ok, entries: {}", entries.len());
            }
            Err(e) => match e.kind() {
                io::ErrorKind::TimedOut | io::ErrorKind::Interrupted => {
                    debug!("expected error: {e}");
                }
                _ => panic!("{:?}", e),
            },
        }
        #[cfg(feature = "time")]
        self.timer_runtime.borrow_mut().wake();
    }
}

impl AsRawFd for RuntimeInner {
    fn as_raw_fd(&self) -> RawFd {
        self.driver.borrow().as_raw_fd()
    }
}

struct RuntimeContext {
    depth: usize,
    ptr: Weak<RuntimeInner>,
}

impl RuntimeContext {
    pub fn new() -> Self {
        Self {
            depth: 0,
            ptr: Weak::new(),
        }
    }

    pub fn inc_depth(&mut self) -> usize {
        let depth = self.depth;
        self.depth += 1;
        depth
    }

    pub fn dec_depth(&mut self) -> usize {
        self.depth -= 1;
        self.depth
    }

    pub fn set_runtime(&mut self, ptr: Weak<RuntimeInner>) -> Weak<RuntimeInner> {
        std::mem::replace(&mut self.ptr, ptr)
    }

    pub fn upgrade_runtime(&self) -> Option<Runtime> {
        self.ptr.upgrade().map(|inner| Runtime { inner })
    }
}

thread_local! {
    static CURRENT_RUNTIME: RefCell<RuntimeContext> = RefCell::new(RuntimeContext::new());
}

/// The async runtime of compio. It is a thread local runtime, and cannot be
/// sent to other threads.
#[derive(Clone)]
pub struct Runtime {
    inner: Rc<RuntimeInner>,
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

    /// Get the current running [`Runtime`].
    pub fn try_current() -> Option<Self> {
        CURRENT_RUNTIME.with_borrow(|r| r.upgrade_runtime())
    }

    /// Get the current running [`Runtime`].
    ///
    /// ## Panics
    ///
    /// This method will panic if there are no running [`Runtime`].
    pub fn current() -> Self {
        Self::try_current().expect("not in a compio runtime")
    }

    pub(crate) fn inner(&self) -> &RuntimeInner {
        &self.inner
    }

    /// Enter the runtime context. This runtime will be set as the `current`
    /// one.
    ///
    /// ## Panics
    ///
    /// When calling `Runtime::enter` multiple times, the returned guards
    /// **must** be dropped in the reverse order that they were acquired.
    /// Failure to do so will result in a panic and possible memory leaks.
    ///
    /// Do **not** do the following, this shows a scenario that will result in a
    /// panic and possible memory leak.
    ///
    /// ```should_panic
    /// use compio_runtime::Runtime;
    ///
    /// let rt1 = Runtime::new().unwrap();
    /// let rt2 = Runtime::new().unwrap();
    ///
    /// let enter1 = rt1.enter();
    /// let enter2 = rt2.enter();
    ///
    /// drop(enter1);
    /// drop(enter2);
    /// ```
    pub fn enter(&self) -> EnterGuard {
        EnterGuard::new(self)
    }

    /// Block on the future till it completes.
    pub fn block_on<F: Future>(&self, future: F) -> F::Output {
        let guard = self.enter();
        guard.block_on(future)
    }

    /// Spawns a new asynchronous task, returning a [`Task`] for it.
    ///
    /// Spawning a task enables the task to execute concurrently to other tasks.
    /// There is no guarantee that a spawned task will execute to completion.
    pub fn spawn<F: Future + 'static>(&self, future: F) -> JoinHandle<F::Output> {
        self.inner.spawn(future)
    }

    /// Spawns a blocking task in a new thread, and wait for it.
    ///
    /// The task will not be cancelled even if the future is dropped.
    pub fn spawn_blocking<T: Send + 'static>(
        &self,
        f: impl (FnOnce() -> T) + Send + Sync + 'static,
    ) -> JoinHandle<T> {
        self.inner.spawn_blocking(f)
    }

    /// Spawns a new asynchronous task, returning a [`Task`] for it.
    ///
    /// # Safety
    ///
    /// The caller should ensure the captured lifetime long enough.
    pub unsafe fn spawn_unchecked<F: Future>(&self, future: F) -> Task<F::Output> {
        self.inner.spawn_unchecked(future)
    }

    /// Low level API to control the runtime.
    ///
    /// Run the scheduled tasks.
    pub fn run(&self) {
        self.inner.run()
    }

    /// Low level API to control the runtime.
    ///
    /// Get the timeout value to be passed to [`Proactor::poll`].
    pub fn current_timeout(&self) -> Option<Duration> {
        self.inner.current_timeout()
    }

    /// Low level API to control the runtime.
    ///
    /// Poll the inner proactor. It is equal to calling [`Runtime::poll_with`]
    /// with [`Runtime::current_timeout`].
    pub fn poll(&self) {
        self.inner.poll()
    }

    /// Low level API to control the runtime.
    ///
    /// Poll the inner proactor with a custom timeout.
    pub fn poll_with(&self, timeout: Option<Duration>) {
        self.inner.poll_with(timeout)
    }

    /// Attach a raw file descriptor/handle/socket to the runtime.
    ///
    /// You only need this when authoring your own high-level APIs. High-level
    /// resources in this crate are attached automatically.
    pub fn attach(&self, fd: RawFd) -> io::Result<()> {
        self.inner.attach(fd)
    }

    /// Submit an operation to the runtime.
    ///
    /// You only need this when authoring your own [`OpCode`].
    pub fn submit<T: OpCode + 'static>(&self, op: T) -> impl Future<Output = BufResult<usize, T>> {
        self.inner.submit(op)
    }
}

impl AsRawFd for Runtime {
    fn as_raw_fd(&self) -> RawFd {
        self.inner.as_raw_fd()
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
        }
    }

    /// Replace proactor builder.
    pub fn with_proactor(&mut self, builder: ProactorBuilder) -> &mut Self {
        self.proactor_builder = builder;
        self
    }

    /// Build [`Runtime`].
    pub fn build(&self) -> io::Result<Runtime> {
        Ok(Runtime {
            inner: Rc::new(RuntimeInner::new(&self.proactor_builder)?),
        })
    }
}

/// Runtime context guard.
///
/// When the guard is dropped, exit the corresponding runtime context.
#[must_use]
pub struct EnterGuard<'a> {
    runtime: &'a Runtime,
    old_ptr: Weak<RuntimeInner>,
    depth: usize,
}

impl<'a> EnterGuard<'a> {
    fn new(runtime: &'a Runtime) -> Self {
        let (old_ptr, depth) = CURRENT_RUNTIME.with_borrow_mut(|ctx| {
            (
                ctx.set_runtime(Rc::downgrade(&runtime.inner)),
                ctx.inc_depth(),
            )
        });
        Self {
            runtime,
            old_ptr,
            depth,
        }
    }

    /// Block on the future in the runtime backed of this guard.
    pub fn block_on<F: Future>(&self, future: F) -> F::Output {
        self.runtime.inner.block_on(future)
    }
}

#[cold]
fn panic_incorrect_drop_order() {
    if !std::thread::panicking() {
        panic!(
            "`EnterGuard` values dropped out of order. Guards returned by `Runtime::enter()` must \
             be dropped in the reverse order as they were acquired."
        )
    }
}

impl Drop for EnterGuard<'_> {
    fn drop(&mut self) {
        let depth = CURRENT_RUNTIME.with_borrow_mut(|ctx| {
            ctx.set_runtime(std::mem::take(&mut self.old_ptr));
            ctx.dec_depth()
        });
        if depth != self.depth {
            panic_incorrect_drop_order()
        }
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
/// by [`Runtime::current`].
pub fn spawn<F: Future + 'static>(future: F) -> JoinHandle<F::Output> {
    Runtime::current().spawn(future)
}

/// Spawns a blocking task in a new thread, and wait for it.
///
/// The task will not be cancelled even if the future is dropped.
///
/// ## Panics
///
/// This method doesn't create runtime. It tries to obtain the current runtime
/// by [`Runtime::current`].
pub fn spawn_blocking<T: Send + 'static>(
    f: impl (FnOnce() -> T) + Send + Sync + 'static,
) -> JoinHandle<T> {
    Runtime::current().spawn_blocking(f)
}
