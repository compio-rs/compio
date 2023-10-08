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
    pub(crate) fn new(n: usize) -> Self {
        let (sender, receiver) = unbounded::<BoxClosure<'static>>();
        let threads = (0..n)
            .map({
                |_| {
                    let receiver = receiver.clone();
                    std::thread::spawn(move || {
                        while let Ok(f) = receiver.recv() {
                            compio_runtime::block_on(f())?;
                        }
                        Ok(())
                    })
                }
            })
            .collect();
        Self { sender, threads }
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
#[derive(Debug)]
pub struct DispatcherBuilder {
    nthreads: usize,
}

impl DispatcherBuilder {
    /// Create a builder with default settings.
    pub fn new() -> Self {
        Self {
            nthreads: available_parallelism().map(|n| n.get()).unwrap_or(1),
        }
    }

    /// Set the number of worker threads of the dispatcher. The default value is
    /// the CPU number. If the CPU number could not be retrieved, the
    /// default value is 1.
    pub fn worker_threads(mut self, nthreads: NonZeroUsize) -> Self {
        self.nthreads = nthreads.get();
        self
    }

    /// Build the [`Dispatcher`].
    pub fn build(self) -> Dispatcher {
        Dispatcher::new(self.nthreads)
    }
}

impl Default for DispatcherBuilder {
    fn default() -> Self {
        Self::new()
    }
}
