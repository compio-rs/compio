//! Multithreading dispatcher for compio.

#![warn(missing_docs)]

use std::{
    any::Any,
    future::Future,
    io,
    num::NonZeroUsize,
    panic::resume_unwind,
    pin::Pin,
    sync::{Arc, Mutex},
    thread::{available_parallelism, JoinHandle},
};

use compio_driver::{AsyncifyPool, ProactorBuilder};
use compio_runtime::{event::Event, Runtime};
use flume::{unbounded, SendError, Sender};

/// The dispatcher. It manages the threads and dispatches the tasks.
pub struct Dispatcher {
    sender: Sender<Box<Closure>>,
    threads: Vec<JoinHandle<()>>,
    pool: AsyncifyPool,
}

impl Dispatcher {
    /// Create the dispatcher with specified number of threads.
    pub(crate) fn new_impl(mut builder: DispatcherBuilder) -> io::Result<Self> {
        let mut proactor_builder = builder.proactor_builder;
        proactor_builder.force_reuse_thread_pool();
        let pool = proactor_builder.create_or_get_thread_pool();
        let (sender, receiver) = unbounded::<Box<Closure>>();

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
                                    let fut = (f)();
                                    if builder.concurrent {
                                        compio_runtime::spawn(fut).detach()
                                    } else {
                                        fut.await
                                    }
                                }
                            })
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

    fn prepare<Fut, Fn, R>(&self, f: Fn) -> (Executing<R>, Box<Closure>)
    where
        Fn: (FnOnce() -> Fut) + Send + 'static,
        Fut: Future<Output = R> + 'static,
        R: Any + Send + 'static,
    {
        let event = Event::new();
        let handle = event.handle();
        let res = Arc::new(Mutex::new(None));
        let dispatched = Executing {
            event,
            result: res.clone(),
        };
        let closure = Box::new(|| {
            Box::pin(async move {
                *res.lock().unwrap() = Some(f().await);
                handle.notify();
            }) as BoxFuture<()>
        });
        (dispatched, closure)
    }

    /// Spawn a boxed closure to the threads.
    ///
    /// If all threads have panicked, this method will return an error with the
    /// sent closure.
    pub fn spawn(&self, closure: Box<Closure>) -> Result<(), SendError<Box<Closure>>> {
        self.sender.send(closure)
    }

    /// Dispatch a task to the threads
    ///
    /// The provided `f` should be [`Send`] because it will be send to another
    /// thread before calling. The return [`Future`] need not to be [`Send`]
    /// because it will be executed on only one thread.
    ///
    /// # Error
    ///
    /// If all threads have panicked, this method will return an error with the
    /// sent closure. Notice that the returned closure is not the same as the
    /// argument and cannot be simply transmuted back to `Fn`.
    pub fn dispatch<Fut, Fn>(&self, f: Fn) -> Result<(), SendError<Box<Closure>>>
    where
        Fn: (FnOnce() -> Fut) + Send + 'static,
        Fut: Future<Output = ()> + 'static,
    {
        self.spawn(Box::new(|| Box::pin(f()) as BoxFuture<()>))
    }

    /// Execute a task on the threads and retrieve its returned value.
    ///
    /// The provided `f` should be [`Send`] because it will be send to another
    /// thread before calling. The return [`Future`] need not to be [`Send`]
    /// because it will be executed on only one thread.
    ///
    /// # Error
    ///
    /// If all threads have panicked, this method will return an error with the
    /// sent closure. Notice that the returned closure is not the same as the
    /// argument and cannot be simply transmuted back to `Fn`.
    pub fn execute<Fut, Fn, R>(&self, f: Fn) -> Result<Executing<R>, SendError<Box<Closure>>>
    where
        Fn: (FnOnce() -> Fut) + Send + 'static,
        Fut: Future<Output = R> + 'static,
        R: Any + Send + 'static,
    {
        let (dispatched, closure) = self.prepare(f);
        self.spawn(closure)?;
        Ok(dispatched)
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
            std::thread::spawn(f);
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

type BoxFuture<T> = Pin<Box<dyn Future<Output = T>>>;
type Closure = dyn (FnOnce() -> BoxFuture<()>) + Send;

/// The join handle for an executing task. It can be used to wait for the
/// task's returned value.
pub struct Executing<R> {
    event: Event,
    result: Arc<Mutex<Option<R>>>,
}

impl<R: 'static> Executing<R> {
    fn take(val: &Mutex<Option<R>>) -> R {
        val.lock()
            .unwrap()
            .take()
            .expect("the result should be set")
    }

    /// Try to wait for the task to complete without blocking.
    pub fn try_join(self) -> Result<R, Self> {
        if self.event.notified() {
            Ok(Self::take(&self.result))
        } else {
            Err(self)
        }
    }

    /// Wait for the task to complete.
    pub async fn join(self) -> R {
        self.event.wait().await;
        Self::take(&self.result)
    }
}
