//! Multithreading dispatcher for compio.

#![warn(missing_docs)]

use std::{
    future::Future,
    io,
    num::NonZeroUsize,
    panic::{resume_unwind, UnwindSafe},
    sync::{Arc, Mutex},
    thread::{available_parallelism, JoinHandle},
};

use compio_driver::{AsyncifyPool, ProactorBuilder};
use compio_runtime::event::{Event, EventHandle};
use crossbeam_channel::{unbounded, Sender};
use futures_util::{future::LocalBoxFuture, FutureExt};

/// The dispatcher. It manages the threads and dispatches the tasks.
pub struct Dispatcher {
    sender: Sender<DispatcherClosure>,
    threads: Vec<JoinHandle<()>>,
    pool: AsyncifyPool,
}

impl Dispatcher {
    /// Create the dispatcher with specified number of threads.
    pub(crate) fn new_impl(mut builder: DispatcherBuilder) -> io::Result<Self> {
        let mut proactor_builder = builder.proactor_builder;
        let pool = proactor_builder.create_or_get_thread_pool();
        // If the reused pool is not set, this call will set it.
        proactor_builder.reuse_thread_pool(pool.clone());

        let (sender, receiver) = unbounded::<DispatcherClosure>();
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
                        compio_runtime::config_proactor(proactor_builder);
                        while let Ok(f) = receiver.recv() {
                            *f.result.lock().unwrap() = Some(std::panic::catch_unwind(move || {
                                compio_runtime::block_on((f.func)());
                            }));
                            f.handle.notify().ok();
                        }
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

    /// Dispatch a task to the threads.
    ///
    /// The provided `f` should be [`Send`] because it will be send to another
    /// thread before calling. The return [`Future`] need not to be [`Send`]
    /// because it will be executed on only one thread.
    pub fn dispatch<
        F: Future<Output = ()> + 'static,
        Fn: (FnOnce() -> F) + Send + UnwindSafe + 'static,
    >(
        &self,
        f: Fn,
    ) -> io::Result<DispatcherJoinHandle> {
        let event = Event::new()?;
        let handle = event.handle()?;
        let join_handle = DispatcherJoinHandle::new(event);
        let closure = DispatcherClosure {
            handle,
            result: join_handle.result.clone(),
            func: Box::new(|| f().boxed_local()),
        };
        self.sender
            .send(closure)
            .expect("the channel should not be disconnected");
        Ok(join_handle)
    }

    /// Stop the dispatcher and wait for the threads to complete. If there is a
    /// thread panicked, this method will resume the panic.
    pub async fn join(self) -> io::Result<()> {
        drop(self.sender);
        let results = Arc::new(Mutex::new(vec![]));
        let event = Event::new()?;
        let handle = event.handle()?;
        if let Err(f) = self.pool.dispatch({
            let results = results.clone();
            move || {
                *results.lock().unwrap() = self
                    .threads
                    .into_iter()
                    .map(|thread| thread.join())
                    .collect();
                handle.notify().ok();
            }
        }) {
            std::thread::spawn(f);
        }
        event.wait().await?;
        let mut guard = results.lock().unwrap();
        for res in std::mem::take::<Vec<std::thread::Result<()>>>(guard.as_mut()) {
            // The thread should not panic.
            res.unwrap_or_else(|e| resume_unwind(e));
        }
        Ok(())
    }
}

/// A builder for [`Dispatcher`].
pub struct DispatcherBuilder {
    nthreads: usize,
    stack_size: Option<usize>,
    names: Option<Box<dyn FnMut(usize) -> String>>,
    proactor_builder: ProactorBuilder,
}

impl DispatcherBuilder {
    /// Create a builder with default settings.
    pub fn new() -> Self {
        Self {
            nthreads: available_parallelism().map(|n| n.get()).unwrap_or(1),
            stack_size: None,
            names: None,
            proactor_builder: ProactorBuilder::new(),
        }
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

type Closure<'a> = dyn (FnOnce() -> LocalBoxFuture<'a, ()>) + Send + UnwindSafe;

struct DispatcherClosure {
    handle: EventHandle,
    result: Arc<Mutex<Option<std::thread::Result<()>>>>,
    func: Box<Closure<'static>>,
}

/// The join handle for dispatched task.
pub struct DispatcherJoinHandle {
    event: Event,
    result: Arc<Mutex<Option<std::thread::Result<()>>>>,
}

impl DispatcherJoinHandle {
    pub(crate) fn new(event: Event) -> Self {
        Self {
            event,
            result: Arc::new(Mutex::new(None)),
        }
    }

    /// Wait for the task to complete.
    pub async fn join(self) -> io::Result<std::thread::Result<()>> {
        self.event.wait().await?;
        Ok(self
            .result
            .lock()
            .unwrap()
            .take()
            .expect("the result should be set"))
    }
}
