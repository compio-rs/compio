//! Multithreading dispatcher.

#![warn(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]
#![doc(
    html_logo_url = "https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-bold.svg"
)]
#![doc(
    html_favicon_url = "https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-bold.svg"
)]

use std::{
    collections::HashSet,
    future::Future,
    io,
    num::NonZeroUsize,
    panic::resume_unwind,
    thread::{JoinHandle, available_parallelism},
};

use compio_driver::{AsyncifyPool, DispatchError, Dispatchable, ProactorBuilder};
use compio_runtime::{JoinHandle as CompioJoinHandle, Runtime};
use flume::{Sender, unbounded};
use futures_channel::oneshot;
#[cfg(unix)]
use libc::{
    SIG_BLOCK, SIG_SETMASK, SIGHUP, SIGINT, SIGPIPE, SIGQUIT, SIGTERM, SIGUSR1, SIGUSR2,
    pthread_sigmask, sigaddset, sigemptyset, sigset_t,
};

type Spawning = Box<dyn Spawnable + Send>;

trait Spawnable {
    fn spawn(self: Box<Self>, handle: &Runtime) -> CompioJoinHandle<()>;
}

/// Concrete type for the closure we're sending to worker threads
struct Concrete<F, R> {
    callback: oneshot::Sender<R>,
    func: F,
}

impl<F, R> Concrete<F, R> {
    pub fn new(func: F) -> (Self, oneshot::Receiver<R>) {
        let (tx, rx) = oneshot::channel();
        (Self { callback: tx, func }, rx)
    }
}

impl<F, Fut, R> Spawnable for Concrete<F, R>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = R>,
    R: Send + 'static,
{
    fn spawn(self: Box<Self>, handle: &Runtime) -> CompioJoinHandle<()> {
        let Concrete { callback, func } = *self;
        handle.spawn(async move {
            let res = func().await;
            callback.send(res).ok();
        })
    }
}

impl<F, R> Dispatchable for Concrete<F, R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    fn run(self: Box<Self>) {
        let Concrete { callback, func } = *self;
        let res = func();
        callback.send(res).ok();
    }
}

/// The dispatcher. It manages the threads and dispatches the tasks.
#[derive(Debug)]
pub struct Dispatcher {
    sender: Sender<Spawning>,
    threads: Vec<JoinHandle<()>>,
    pool: AsyncifyPool,
}

impl Dispatcher {
    /// Create the dispatcher with specified number of threads.
    pub(crate) fn new_impl(builder: DispatcherBuilder) -> io::Result<Self> {
        let DispatcherBuilder {
            nthreads,
            concurrent,
            #[cfg(unix)]
            block_signals,
            stack_size,
            mut thread_affinity,
            mut names,
            mut proactor_builder,
        } = builder;
        proactor_builder.force_reuse_thread_pool();
        let pool = proactor_builder.create_or_get_thread_pool();
        let (sender, receiver) = unbounded::<Spawning>();

        // Block standard signals before spawning workers.
        #[cfg(unix)]
        let old_sigmask = if block_signals {
            Some(unsafe {
                let mut new_mask: sigset_t = std::mem::zeroed();
                sigemptyset(&mut new_mask);
                sigaddset(&mut new_mask, SIGINT);
                sigaddset(&mut new_mask, SIGTERM);
                sigaddset(&mut new_mask, SIGQUIT);
                sigaddset(&mut new_mask, SIGHUP);
                sigaddset(&mut new_mask, SIGUSR1);
                sigaddset(&mut new_mask, SIGUSR2);
                sigaddset(&mut new_mask, SIGPIPE);

                let mut old_mask: sigset_t = std::mem::zeroed();
                pthread_sigmask(SIG_BLOCK, &new_mask, &mut old_mask);
                old_mask
            })
        } else {
            None
        };

        let threads = (0..nthreads)
            .map({
                |index| {
                    let proactor_builder = proactor_builder.clone();
                    let receiver = receiver.clone();

                    let thread_builder = std::thread::Builder::new();
                    let thread_builder = if let Some(s) = stack_size {
                        thread_builder.stack_size(s)
                    } else {
                        thread_builder
                    };
                    let thread_builder = if let Some(f) = &mut names {
                        thread_builder.name(f(index))
                    } else {
                        thread_builder
                    };

                    let cpus = if let Some(f) = &mut thread_affinity {
                        f(index)
                    } else {
                        HashSet::new()
                    };
                    thread_builder.spawn(move || {
                        Runtime::builder()
                            .with_proactor(proactor_builder)
                            .thread_affinity(cpus)
                            .build()
                            .expect("cannot create compio runtime")
                            .block_on(async move {
                                while let Ok(f) = receiver.recv_async().await {
                                    let task = Runtime::with_current(|rt| f.spawn(rt));
                                    if concurrent {
                                        task.detach()
                                    } else {
                                        task.await.ok();
                                    }
                                }
                            });
                    })
                }
            })
            .collect::<io::Result<Vec<_>>>()?;

        // Restore the original signal mask.
        #[cfg(unix)]
        if let Some(old_mask) = old_sigmask {
            unsafe {
                pthread_sigmask(SIG_SETMASK, &old_mask, std::ptr::null_mut());
            }
        }

        Ok(Self {
            sender,
            threads,
            pool,
        })
    }

    /// Create the dispatcher with default config.
    pub fn new() -> io::Result<Self> {
        Self::builder().build()
    }

    /// Create a builder to build a dispatcher.
    pub fn builder() -> DispatcherBuilder {
        DispatcherBuilder::default()
    }

    /// Dispatch a task to the threads
    ///
    /// The provided `f` should be [`Send`] because it will be send to another
    /// thread before calling. The returned [`Future`] need not to be [`Send`]
    /// because it will be executed on only one thread.
    ///
    /// # Error
    ///
    /// If all threads have panicked, this method will return an error with the
    /// sent closure.
    pub fn dispatch<Fn, Fut, R>(&self, f: Fn) -> Result<oneshot::Receiver<R>, DispatchError<Fn>>
    where
        Fn: (FnOnce() -> Fut) + Send + 'static,
        Fut: Future<Output = R> + 'static,
        R: Send + 'static,
    {
        let (concrete, rx) = Concrete::new(f);

        match self.sender.send(Box::new(concrete)) {
            Ok(_) => Ok(rx),
            Err(err) => {
                // SAFETY: We know the dispatchable we sent has type `Concrete<Fn, R>`
                let recovered =
                    unsafe { Box::from_raw(Box::into_raw(err.0) as *mut Concrete<Fn, R>) };
                Err(DispatchError(recovered.func))
            }
        }
    }

    /// Dispatch a blocking task to the threads.
    ///
    /// Blocking pool of the dispatcher will be obtained from the proactor
    /// builder. So any configuration of the proactor's blocking pool will be
    /// applied to the dispatcher.
    ///
    /// # Error
    ///
    /// If all threads are busy and the thread pool is full, this method will
    /// return an error with the original closure. The limit can be configured
    /// with [`DispatcherBuilder::proactor_builder`] and
    /// [`ProactorBuilder::thread_pool_limit`].
    pub fn dispatch_blocking<Fn, R>(&self, f: Fn) -> Result<oneshot::Receiver<R>, DispatchError<Fn>>
    where
        Fn: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        let (concrete, rx) = Concrete::new(f);

        self.pool
            .dispatch(concrete)
            .map_err(|e| DispatchError(e.0.func))?;

        Ok(rx)
    }

    /// Stop the dispatcher and wait for the threads to complete. If there is a
    /// thread panicked, this method will resume the panic.
    pub async fn join(self) -> io::Result<()> {
        drop(self.sender);
        let (tx, rx) = oneshot::channel::<Vec<_>>();
        if let Err(f) = self.pool.dispatch({
            move || {
                let results = self
                    .threads
                    .into_iter()
                    .map(|thread| thread.join())
                    .collect();
                tx.send(results).ok();
            }
        }) {
            std::thread::spawn(f.0);
        }
        let results = rx
            .await
            .map_err(|_| io::Error::other("the join task cancelled unexpectedly"))?;
        for res in results {
            res.unwrap_or_else(|e| resume_unwind(e));
        }
        Ok(())
    }
}

/// A builder for [`Dispatcher`].
pub struct DispatcherBuilder {
    nthreads: usize,
    concurrent: bool,
    #[cfg(unix)]
    block_signals: bool,
    stack_size: Option<usize>,
    thread_affinity: Option<Box<dyn FnMut(usize) -> HashSet<usize>>>,
    names: Option<Box<dyn FnMut(usize) -> String>>,
    proactor_builder: ProactorBuilder,
}

impl DispatcherBuilder {
    /// Create a builder with default settings.
    pub fn new() -> Self {
        Self {
            nthreads: available_parallelism().map(|n| n.get()).unwrap_or(1),
            concurrent: true,
            #[cfg(unix)]
            block_signals: true,
            stack_size: None,
            thread_affinity: None,
            names: None,
            proactor_builder: ProactorBuilder::new(),
        }
    }

    /// If execute tasks concurrently. Default to be `true`.
    ///
    /// When set to `false`, tasks are executed sequentially without any
    /// concurrency within the thread.
    pub fn concurrent(mut self, concurrent: bool) -> Self {
        self.concurrent = concurrent;
        self
    }

    /// Set the number of worker threads of the dispatcher. The default value is
    /// the CPU number. If the CPU number could not be retrieved, the
    /// default value is 1.
    pub fn worker_threads(mut self, nthreads: NonZeroUsize) -> Self {
        self.nthreads = nthreads.get();
        self
    }

    /// Set the size of stack of the worker threads.
    pub fn stack_size(mut self, s: usize) -> Self {
        self.stack_size = Some(s);
        self
    }

    /// Block standard signals on worker threads. Default to be `true`.
    ///
    /// When enabled, `SIGINT`, `SIGTERM`, `SIGQUIT`, `SIGHUP`, `SIGUSR1`,
    /// `SIGUSR2`, and `SIGPIPE` are masked on worker threads.
    #[cfg(unix)]
    pub fn block_signals(mut self, block_signals: bool) -> Self {
        self.block_signals = block_signals;
        self
    }

    /// Set the thread affinity for the dispatcher.
    pub fn thread_affinity(mut self, f: impl FnMut(usize) -> HashSet<usize> + 'static) -> Self {
        self.thread_affinity = Some(Box::new(f));
        self
    }

    /// Provide a function to assign names to the worker threads.
    pub fn thread_names(mut self, f: impl (FnMut(usize) -> String) + 'static) -> Self {
        self.names = Some(Box::new(f) as _);
        self
    }

    /// Set the proactor builder for the inner runtimes.
    pub fn proactor_builder(mut self, builder: ProactorBuilder) -> Self {
        self.proactor_builder = builder;
        self
    }

    /// Build the [`Dispatcher`].
    pub fn build(self) -> io::Result<Dispatcher> {
        Dispatcher::new_impl(self)
    }
}

impl Default for DispatcherBuilder {
    fn default() -> Self {
        Self::new()
    }
}
