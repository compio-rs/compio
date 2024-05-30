//! Multithreading dispatcher for compio.

#![warn(missing_docs)]

use std::{
    future::Future,
    io,
    num::NonZeroUsize,
    panic::resume_unwind,
    sync::{Arc, Mutex},
    thread::{available_parallelism, JoinHandle},
};

use compio_driver::{AsyncifyPool, DispatchError, Dispatchable, ProactorBuilder};
use compio_runtime::{event::Event, JoinHandle as CompioJoinHandle, Runtime};
use flume::{unbounded, Sender};
use futures_channel::oneshot;

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
    pub(crate) fn new_impl(mut builder: DispatcherBuilder) -> io::Result<Self> {
        let mut proactor_builder = builder.proactor_builder;
        proactor_builder.force_reuse_thread_pool();
        let pool = proactor_builder.create_or_get_thread_pool();
        let (sender, receiver) = unbounded::<Spawning>();

        let threads = (0..builder.nthreads)
            .map({
                |index| {
                    let proactor_builder = proactor_builder.clone();
                    let receiver = receiver.clone();

                    let thread_builder = std::thread::Builder::new();
                    let thread_builder = if let Some(s) = builder.stack_size {
                        thread_builder.stack_size(s)
                    } else {
                        thread_builder
                    };
                    let thread_builder = if let Some(f) = &mut builder.names {
                        thread_builder.name(f(index))
                    } else {
                        thread_builder
                    };

                    thread_builder.spawn(move || {
                        Runtime::builder()
                            .with_proactor(proactor_builder)
                            .build()
                            .expect("cannot create compio runtime")
                            .block_on(async move {
                                while let Ok(f) = receiver.recv_async().await {
                                    let task = Runtime::with_current(|rt| f.spawn(rt));
                                    if builder.concurrent {
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
        let results = Arc::new(Mutex::new(vec![]));
        let event = Event::new();
        let handle = event.handle();
        if let Err(f) = self.pool.dispatch({
            let results = results.clone();
            move || {
                *results.lock().unwrap() = self
                    .threads
                    .into_iter()
                    .map(|thread| thread.join())
                    .collect();
                handle.notify();
            }
        }) {
            std::thread::spawn(f.0);
        }
        event.wait().await;
        let mut guard = results.lock().unwrap();
        for res in std::mem::take::<Vec<std::thread::Result<()>>>(guard.as_mut()) {
            res.unwrap_or_else(|e| resume_unwind(e));
        }
        Ok(())
    }
}

/// A builder for [`Dispatcher`].
pub struct DispatcherBuilder {
    nthreads: usize,
    concurrent: bool,
    stack_size: Option<usize>,
    names: Option<Box<dyn FnMut(usize) -> String>>,
    proactor_builder: ProactorBuilder,
}

impl DispatcherBuilder {
    /// Create a builder with default settings.
    pub fn new() -> Self {
        Self {
            nthreads: available_parallelism().map(|n| n.get()).unwrap_or(1),
            concurrent: true,
            stack_size: None,
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
