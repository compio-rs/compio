//! The compio runtime.
//!
//! ```
//! let ans = compio_runtime::Runtime::new().unwrap().block_on(async {
//!     println!("Hello world!");
//!     42
//! });
//! assert_eq!(ans, 42);
//! ```

#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(feature = "current_thread_id", feature(current_thread_id))]
#![allow(unused_features)]
#![warn(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]
#![doc(
    html_logo_url = "https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-bold.svg"
)]
#![doc(
    html_favicon_url = "https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-bold.svg"
)]

mod affinity;
mod attacher;
mod cancel;
mod future;
mod waker;

pub mod fd;

#[cfg(feature = "time")]
pub mod time;

use std::{
    cell::RefCell,
    collections::HashSet,
    fmt::Debug,
    future::Future,
    io,
    rc::Rc,
    task::{Context, Poll, Waker},
    time::Duration,
};

use compio_buf::{BufResult, IntoInner};
use compio_driver::{AsRawFd, DriverType, OpCode, Proactor, ProactorBuilder, RawFd, op::Asyncify};
pub use compio_driver::{BufferPool, ErrorExt};
use compio_executor::{Executor, ExecutorConfig};
pub use compio_executor::{JoinHandle, ResumeUnwind};
use compio_log::{debug, instrument};

use crate::affinity::bind_to_cpu_set;
#[cfg(feature = "time")]
use crate::time::TimerRuntime;
pub use crate::{attacher::*, cancel::CancelToken, future::*};

scoped_tls::scoped_thread_local!(static CURRENT_RUNTIME: Runtime);

#[cold]
fn not_in_compio_runtime() -> ! {
    panic!("not in a compio runtime")
}

/// The async runtime of compio.
///
/// It is a thread-local runtime, meaning it cannot be sent to other threads.
#[derive(Clone)]
pub struct Runtime {
    executor: Rc<Executor>,
    driver: Rc<RefCell<Proactor>>,
    #[cfg(feature = "time")]
    timer_runtime: Rc<RefCell<TimerRuntime>>,
}

impl Debug for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = f.debug_struct("Runtime");
        s.field("executor", &self.executor);
        s.field("driver", &"...");
        #[cfg(feature = "time")]
        s.field("timer_runtime", &"...");
        s.finish()
    }
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
    /// This method will panic if there is no running [`Runtime`].
    pub fn with_current<T, F: FnOnce(&Self) -> T>(f: F) -> T {
        if CURRENT_RUNTIME.is_set() {
            CURRENT_RUNTIME.with(f)
        } else {
            not_in_compio_runtime()
        }
    }

    /// Try to get the current runtime, and if no runtime is running, return
    /// `None`.
    pub fn try_current() -> Option<Self> {
        if CURRENT_RUNTIME.is_set() {
            Some(CURRENT_RUNTIME.with(|r| r.clone()))
        } else {
            None
        }
    }

    /// Get the current runtime.
    ///
    /// # Panics
    ///
    /// This method will panic if there is no running [`Runtime`].
    pub fn current() -> Self {
        if CURRENT_RUNTIME.is_set() {
            CURRENT_RUNTIME.with(|r| r.clone())
        } else {
            not_in_compio_runtime()
        }
    }

    /// Set this runtime as current runtime, and perform a function in the
    /// current scope.
    pub fn enter<T, F: FnOnce() -> T>(&self, f: F) -> T {
        CURRENT_RUNTIME.set(self, f)
    }

    /// Low level API to control the runtime.
    ///
    /// Run the scheduled tasks.
    ///
    /// The return value indicates whether there are still tasks in the queue.
    pub fn run(&self) -> bool {
        self.executor.tick()
    }

    /// Low level API to control the runtime.
    ///
    /// Create a waker that always notifies the runtime when woken.
    pub fn waker(&self) -> Waker {
        self.driver.borrow().waker()
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

    /// Spawns a new asynchronous task, returning a [`JoinHandle`] for it.
    ///
    /// Spawning a task enables the task to execute concurrently to other tasks.
    /// There is no guarantee that a spawned task will execute to completion.
    pub fn spawn<F: Future + 'static>(&self, future: F) -> JoinHandle<F::Output> {
        self.executor.spawn(future)
    }

    /// Spawns a blocking task in a new thread, and wait for it.
    ///
    /// The task will not be cancelled even if the future is dropped.
    pub fn spawn_blocking<T: Send + 'static>(
        &self,
        f: impl (FnOnce() -> T) + Send + 'static,
    ) -> JoinHandle<T> {
        use futures_util::FutureExt;

        let op = Asyncify::new(move || {
            // TODO: Refactor blocking pool and handle panic within worker and propagate it
            // back
            let res = f();
            BufResult(Ok(0), res)
        });
        let submit = self.submit(op);
        self.spawn(submit.map(|res| res.1.into_inner()))
    }

    /// Attach a raw file descriptor/handle/socket to the runtime.
    ///
    /// You only need this when authoring your own high-level APIs. High-level
    /// resources in this crate are attached automatically.
    pub fn attach(&self, fd: RawFd) -> io::Result<()> {
        self.driver.borrow_mut().attach(fd)
    }

    /// Submit an operation to the runtime.
    ///
    /// You only need this when authoring your own [`OpCode`].
    pub fn submit<T: OpCode + 'static>(&self, op: T) -> Submit<T> {
        Submit::new(self.driver.clone(), op)
    }

    /// Submit a multishot operation to the runtime.
    ///
    /// You only need this when authoring your own [`OpCode`].
    pub fn submit_multi<T: OpCode + 'static>(&self, op: T) -> SubmitMulti<T> {
        SubmitMulti::new(self.driver.clone(), op)
    }

    /// Flush the driver and return whether the driver has been notified.
    ///
    /// See [`Proactor::flush`] for more details.
    pub fn flush(&self) -> bool {
        self.driver.borrow_mut().flush()
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

    /// Get buffer pool of the runtime.
    ///
    /// This will lazily initialize the pool at the first time it's accessed,
    /// and future access to the pool will be cheap and infallible.
    pub fn buffer_pool(&self) -> io::Result<BufferPool> {
        self.driver.borrow_mut().buffer_pool()
    }

    /// Register file descriptors for fixed-file operations.
    ///
    /// This is only supported on io-uring driver, and will return an
    /// [`Unsupported`] io error on all other drivers.
    ///
    /// [`Unsupported`]: std::io::ErrorKind::Unsupported
    pub fn register_files(&self, fds: &[RawFd]) -> io::Result<()> {
        self.driver.borrow_mut().register_files(fds)
    }

    /// Unregister previously registered file descriptors.
    ///
    /// This is only supported on io-uring driver, and will return an
    /// [`Unsupported`] io error on all other drivers.
    ///
    /// [`Unsupported`]: std::io::ErrorKind::Unsupported
    pub fn unregister_files(&self) -> io::Result<()> {
        self.driver.borrow_mut().unregister_files()
    }

    /// Register the personality for the runtime.
    ///
    /// This is only supported on io-uring driver, and will return an
    /// [`Unsupported`] io error on all other drivers.
    ///
    /// The returned personality can be used with
    /// [`FutureExt::with_personality`].
    ///
    /// [`Unsupported`]: std::io::ErrorKind::Unsupported
    pub fn register_personality(&self) -> io::Result<u16> {
        self.driver.borrow_mut().register_personality()
    }

    /// Unregister the given personality for the runtime.
    ///
    /// This is only supported on io-uring driver, and will return an
    /// [`Unsupported`] io error on all other drivers.
    ///
    /// [`Unsupported`]: std::io::ErrorKind::Unsupported
    pub fn unregister_personality(&self, personality: u16) -> io::Result<()> {
        self.driver.borrow_mut().unregister_personality(personality)
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        // this is not the last runtime reference, no need to clear
        if Rc::strong_count(&self.executor) > 1 {
            return;
        }

        self.enter(|| {
            self.executor.clear();
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
    sync_queue_size: usize,
    local_queue_size: usize,
    event_interval: u32,
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
            sync_queue_size: 64,
            local_queue_size: 64,
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
        self.event_interval = val as _;
        self
    }

    /// The size of the sync queue, which is used to wake up tasks from other
    /// threads (remote).
    ///
    /// This is fixed and will create backpressure in other remote threads when
    /// full.
    pub fn sync_queue_size(&mut self, val: usize) -> &mut Self {
        self.sync_queue_size = val;
        self
    }

    /// The size of the local queues, which is used to wake up tasks within the
    /// same thread.
    ///
    /// This is dynamically resized to avoid blocking.
    pub fn local_queue_size(&mut self, val: usize) -> &mut Self {
        self.local_queue_size = val;
        self
    }

    /// Build [`Runtime`].
    pub fn build(&self) -> io::Result<Runtime> {
        let RuntimeBuilder {
            proactor_builder,
            thread_affinity,
            sync_queue_size,
            local_queue_size,
            event_interval,
        } = self;

        if !thread_affinity.is_empty() {
            bind_to_cpu_set(thread_affinity);
        }
        let driver = proactor_builder.build()?;
        let executor = Executor::with_config(ExecutorConfig {
            max_interval: *event_interval,
            sync_queue_size: *sync_queue_size,
            local_queue_size: *local_queue_size,
            waker: Some(driver.waker()),
        });
        Ok(Runtime {
            executor: Rc::new(executor),
            driver: Rc::new(RefCell::new(driver)),
            #[cfg(feature = "time")]
            timer_runtime: Rc::new(RefCell::new(TimerRuntime::new())),
        })
    }
}

/// Spawns a new asynchronous task, returning a [`JoinHandle`] for it.
///
/// Spawning a task enables the task to execute concurrently to other tasks.
/// There is no guarantee that a spawned task will execute to completion.
///
/// ```
/// # compio_runtime::Runtime::new().unwrap().block_on(async {
/// use compio_runtime::ResumeUnwind;
///
/// let task = compio_runtime::spawn(async {
///     println!("Hello from a spawned task!");
///     42
/// });
///
/// assert_eq!(
///     task.await.resume_unwind().expect("shouldn't be cancelled"),
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
/// This method doesn't create runtime and will panic if it's not within a
/// runtime. It tries to obtain the current runtime with
/// [`Runtime::with_current`].
pub fn submit<T: OpCode + 'static>(op: T) -> Submit<T> {
    Runtime::with_current(|r| r.submit(op))
}

/// Submit a multishot operation to the current runtime, and return a stream for
/// it.
///
/// ## Panics
///
/// This method doesn't create runtime and will panic if it's not within a
/// runtime. It tries to obtain the current runtime with
/// [`Runtime::with_current`].
pub fn submit_multi<T: OpCode + 'static>(op: T) -> SubmitMulti<T> {
    Runtime::with_current(|r| r.submit_multi(op))
}

/// Register file descriptors for fixed-file operations with the current
/// runtime's io_uring instance.
///
/// This only works on `io_uring` driver. It will return an [`Unsupported`]
/// error on other drivers.
///
/// ## Panics
///
/// This method doesn't create runtime. It tries to obtain the current runtime
/// by [`Runtime::with_current`].
///
/// [`Unsupported`]: std::io::ErrorKind::Unsupported
pub fn register_files(fds: &[RawFd]) -> io::Result<()> {
    Runtime::with_current(|r| r.register_files(fds))
}

/// Unregister previously registered file descriptors from the current
/// runtime's io_uring instance.
///
/// This only works on `io_uring` driver. It will return an [`Unsupported`]
/// error on other drivers.
///
/// ## Panics
///
/// This method doesn't create runtime. It tries to obtain the current runtime
/// by [`Runtime::with_current`].
///
/// [`Unsupported`]: std::io::ErrorKind::Unsupported
pub fn unregister_files() -> io::Result<()> {
    Runtime::with_current(|r| r.unregister_files())
}
