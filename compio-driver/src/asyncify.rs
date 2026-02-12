use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use crossfire::mpmc;

type BoxedDispatchable = Box<dyn Dispatchable + Send>;

type Sender = crossfire::MTx<mpmc::Array<BoxedDispatchable>>;
type Receiver = crossfire::MRx<mpmc::Array<BoxedDispatchable>>;

/// An error that may be emitted when all worker threads are busy. It simply
/// returns the dispatchable value with a convenient [`fmt::Debug`] and
/// [`fmt::Display`] implementation.
#[derive(Copy, Clone, PartialEq, Eq)]
pub struct DispatchError<T>(pub T);

impl<T> DispatchError<T> {
    /// Consume the error, yielding the dispatchable that failed to be sent.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> fmt::Debug for DispatchError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        "DispatchError(..)".fmt(f)
    }
}

impl<T> fmt::Display for DispatchError<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        "all threads are busy".fmt(f)
    }
}

impl<T> std::error::Error for DispatchError<T> {}

/// A trait for dispatching a closure. It's implemented for all `FnOnce() + Send
/// + 'static` but may also be implemented for any other types that are `Send`
///   and `'static`.
pub trait Dispatchable: Send + 'static {
    /// Run the dispatchable
    fn run(self: Box<Self>);
}

impl<F> Dispatchable for F
where
    F: FnOnce() + Send + 'static,
{
    fn run(self: Box<Self>) {
        (*self)()
    }
}

struct TotalGuard(Arc<AtomicUsize>);

impl Drop for TotalGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

fn worker(
    receiver: Receiver,
    total_counter: Arc<AtomicUsize>,
    idle_counter: Arc<AtomicUsize>,
    timeout: Duration,
) -> impl FnOnce() {
    move || {
        // total_counter already incremented by dispatcher
        let _guard = TotalGuard(total_counter);
        loop {
            idle_counter.fetch_add(1, Ordering::SeqCst);
            let res = receiver.recv_timeout(timeout);
            idle_counter.fetch_sub(1, Ordering::SeqCst);
            match res {
                Ok(f) => f.run(),
                Err(_) => break,
            }
        }
    }
}

/// A thread pool to perform blocking operations in other threads.
#[derive(Debug, Clone)]
pub struct AsyncifyPool {
    sender: Sender,
    receiver: Receiver,
    total_counter: Arc<AtomicUsize>,
    idle_counter: Arc<AtomicUsize>,
    thread_limit: usize,
    recv_timeout: Duration,
}

impl AsyncifyPool {
    /// Create [`AsyncifyPool`] with thread number limit and channel receive
    /// timeout.
    pub fn new(thread_limit: usize, recv_timeout: Duration) -> Self {
        let (sender, receiver) = mpmc::bounded_blocking(1);
        Self {
            sender,
            receiver,
            total_counter: Arc::new(AtomicUsize::new(0)),
            idle_counter: Arc::new(AtomicUsize::new(0)),
            thread_limit,
            recv_timeout,
        }
    }

    /// Send a dispatchable, usually a closure, to another thread. Usually the
    /// user should not use it. When all threads are busy and thread number
    /// limit has been reached, it will return an error with the original
    /// dispatchable.
    pub fn dispatch<D: Dispatchable>(&self, f: D) -> Result<(), DispatchError<D>> {
        let thread_limit = self.thread_limit;
        if thread_limit == 0 {
            panic!("the thread pool is needed but no worker thread is running");
        }

        // Fast path: if anyone is idle, try sending directly.
        if self.idle_counter.load(Ordering::SeqCst) > 0 {
            match self.sender.try_send(Box::new(f) as BoxedDispatchable) {
                Ok(_) => return Ok(()),
                Err(crossfire::TrySendError::Full(f)) => {
                    // This is possible if multiple dispatchers raced for the idle worker.
                    // Fall through to slow path.
                    return self.dispatch_slow_internal(f).map_err(|e| {
                        // SAFETY: we can ensure the type
                        DispatchError(*unsafe { Box::from_raw(Box::into_raw(e.0).cast()) })
                    });
                }
                Err(crossfire::TrySendError::Disconnected(_)) => unreachable!(),
            }
        }

        self.dispatch_slow_internal(Box::new(f) as BoxedDispatchable)
            .map_err(|e| {
                // SAFETY: we can ensure the type
                DispatchError(*unsafe { Box::from_raw(Box::into_raw(e.0).cast()) })
            })
    }

    fn dispatch_slow_internal(
        &self,
        f: BoxedDispatchable,
    ) -> Result<(), DispatchError<BoxedDispatchable>> {
        let thread_limit = self.thread_limit;
        let total = self.total_counter.load(Ordering::SeqCst);

        if total < thread_limit {
            // Under limit, we can spawn a worker.
            // We increment total_counter BEFORE spawn to prevent over-spawning.
            self.total_counter.fetch_add(1, Ordering::SeqCst);
            std::thread::spawn(worker(
                self.receiver.clone(),
                self.total_counter.clone(),
                self.idle_counter.clone(),
                self.recv_timeout,
            ));
            // After spawning, the buffer might still be full but the new worker
            // will soon clear it. We use a blocking send here to be sure.
            self.sender.send(f).ok();
            Ok(())
        } else {
            // At limit and no one is idle.
            // One last try_send in case someone just became idle.
            match self.sender.try_send(f) {
                Ok(_) => Ok(()),
                Err(crossfire::TrySendError::Full(f)) => Err(DispatchError(f)),
                Err(crossfire::TrySendError::Disconnected(_)) => unreachable!(),
            }
        }
    }
}
