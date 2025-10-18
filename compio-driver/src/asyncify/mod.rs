use std::{self, fmt, io, time::Duration};

cfg_if::cfg_if! {
    if #[cfg(windows)] {
        #[path = "windows.rs"]
        mod sys;
    } else {
        #[path = "fallback.rs"]
        mod sys;
    }
}

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

/// A thread pool to perform blocking operations in other threads.
#[derive(Debug, Clone)]
pub struct AsyncifyPool(sys::AsyncifyPool);

impl AsyncifyPool {
    /// Create [`AsyncifyPool`] with thread number limit and channel receive
    /// timeout.
    pub fn new(thread_limit: usize, recv_timeout: Duration) -> io::Result<Self> {
        Ok(Self(sys::AsyncifyPool::new(thread_limit, recv_timeout)?))
    }

    /// Send a dispatchable, usually a closure, to another thread. Usually the
    /// user should not use it. When all threads are busy and thread number
    /// limit has been reached, it will return an error with the original
    /// dispatchable.
    pub fn dispatch<D: Dispatchable>(&self, f: D) -> Result<(), DispatchError<D>> {
        self.0.dispatch(f)
    }

    #[cfg(windows)]
    pub(crate) fn as_ptr(
        &self,
    ) -> *const windows_sys::Win32::System::Threading::TP_CALLBACK_ENVIRON_V3 {
        self.0.as_ptr()
    }
}
