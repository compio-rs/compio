//! Multithreading dispatcher for compio.

#![warn(missing_docs)]

use std::{
    future::Future,
    io,
    num::NonZeroUsize,
    panic::resume_unwind,
    thread::{available_parallelism, JoinHandle},
};

use crossbeam_channel::{unbounded, SendError, Sender};
use futures_util::{future::LocalBoxFuture, FutureExt};

type BoxClosure<'a> = Box<dyn (FnOnce() -> LocalBoxFuture<'a, io::Result<()>>) + Send>;

/// The dispatcher. It manages the threads and dispatches the tasks.
pub struct Dispatcher {
    sender: Sender<BoxClosure<'static>>,
    threads: Vec<JoinHandle<io::Result<()>>>,
}

impl Dispatcher {
    /// Create the dispatcher with specified number of threads.
    pub(crate) fn new_impl(
        n: usize,
        stack_size: Option<usize>,
        mut names: Option<Box<dyn FnMut(usize) -> String>>,
    ) -> io::Result<Self> {
        let (sender, receiver) = unbounded::<BoxClosure<'static>>();
        let threads = (0..n)
            .map({
                |index| {
                    let receiver = receiver.clone();

                    let builder = std::thread::Builder::new();
                    let builder = if let Some(s) = stack_size {
                        builder.stack_size(s)
                    } else {
                        builder
                    };
                    let builder = if let Some(f) = &mut names {
                        builder.name(f(index))
                    } else {
                        builder
                    };

                    builder.spawn(move || {
                        while let Ok(f) = receiver.recv() {
                            compio_runtime::block_on(f())?;
                        }
                        Ok(())
                    })
                }
            })
            .collect::<io::Result<Vec<_>>>()?;
        Ok(Self { sender, threads })
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
    pub fn dispatch<F: Future<Output = io::Result<()>> + 'static>(
        &self,
        f: impl (FnOnce() -> F) + Send + 'static,
    ) -> Result<(), SendError<BoxClosure<'static>>> {
        self.sender
            .send(Box::new(move || f().boxed_local()) as BoxClosure<'static>)
    }

    /// Stop the dispatcher and wait for the threads to complete. If there is a
    /// thread panicked, this method will resume the panic.
    pub fn join(self) -> Vec<io::Result<()>> {
        drop(self.sender);
        self.threads
            .into_iter()
            .map(|thread| thread.join().unwrap_or_else(|e| resume_unwind(e)))
            .collect()
    }
}

/// A builder for [`Dispatcher`].
pub struct DispatcherBuilder {
    nthreads: usize,
    stack_size: Option<usize>,
    names: Option<Box<dyn FnMut(usize) -> String>>,
}

impl DispatcherBuilder {
    /// Create a builder with default settings.
    pub fn new() -> Self {
        Self {
            nthreads: available_parallelism().map(|n| n.get()).unwrap_or(1),
            stack_size: None,
            names: None,
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

    /// Build the [`Dispatcher`].
    pub fn build(self) -> io::Result<Dispatcher> {
        Dispatcher::new_impl(self.nthreads, self.stack_size, self.names)
    }
}

impl Default for DispatcherBuilder {
    fn default() -> Self {
        Self::new()
    }
}
