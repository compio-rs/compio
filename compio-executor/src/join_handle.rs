use std::{
    error::Error,
    fmt::Display,
    marker::PhantomData,
    mem::ManuallyDrop,
    panic::resume_unwind,
    pin::Pin,
    ptr,
    task::{Context, Poll},
};

use compio_log::{instrument, trace};

use crate::{Panic, task::Task};

/// A handle that awaits the result of a task.
///
/// Dropping a [`JoinHandle`] will cancel the task. To run the task in the
/// background, use [`JoinHandle::detach`].
#[must_use = "Drop `JoinHandle` will cancel the task. Use `detach` to run it in background."]
#[derive(Debug)]
#[repr(transparent)]
pub struct JoinHandle<T> {
    task: Option<Task>,
    _marker: PhantomData<T>,
}

/// If T is send, we can poll result from other thread
unsafe impl<T: Send> Send for JoinHandle<T> {}

/// JoinHandle does not expose any &self interface, so it's unconditionally
/// Sync.
unsafe impl<T> Sync for JoinHandle<T> {}

impl<T> Unpin for JoinHandle<T> {}

impl<T> JoinHandle<T> {
    pub(crate) fn new(task: Task) -> Self {
        Self {
            task: Some(task),
            _marker: PhantomData,
        }
    }

    /// Cancel the task and wait for the result, if any.
    pub async fn cancel(self) -> Option<T> {
        self.task.as_ref()?.cancel(false);
        self.await.ok()
    }

    /// Detach the task to let it run in the background.
    pub fn detach(self) {
        unsafe { ptr::drop_in_place(&raw mut ManuallyDrop::new(self).task) };
    }
}

/// Task failed to execute to completion.
#[derive(Debug)]
pub enum JoinError {
    /// The task was cancelled.
    Cancelled,
    /// The task panicked.
    Panicked(Panic),
}

/// Trait to resume unwind from a [`JoinError`].
pub trait ResumeUnwind {
    /// The output type.
    type Output;

    /// Resume the panic if the task panicked.
    fn resume_unwind(self) -> Self::Output;
}

impl<T> ResumeUnwind for Result<T, JoinError> {
    type Output = Option<T>;

    fn resume_unwind(self) -> Self::Output {
        match self {
            Ok(res) => Some(res),
            Err(JoinError::Cancelled) => None,
            Err(JoinError::Panicked(e)) => resume_unwind(e),
        }
    }
}

impl JoinError {
    /// Resume unwind if the task panicked, otherwise do nothing.
    pub fn resume_unwind(self) {
        if let JoinError::Panicked(e) = self {
            resume_unwind(e)
        }
    }
}

impl Display for JoinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JoinError::Cancelled => write!(f, "Task was cancelled"),
            JoinError::Panicked(_) => write!(f, "Task has panicked"),
        }
    }
}

impl Error for JoinError {}

impl<T> Future for JoinHandle<T> {
    type Output = Result<T, JoinError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        instrument!(compio_log::Level::TRACE, "JoinHandle::poll");

        let task = self.task.as_ref().expect("Cannot poll after completion");

        unsafe { task.poll(cx) }.map(|res| {
            trace!("Poll ready");

            self.task = None;

            match res {
                Some(Ok(res)) => Ok(res),
                Some(Err(e)) => Err(JoinError::Panicked(e)),
                None => Err(JoinError::Cancelled),
            }
        })
    }
}

impl<T> Drop for JoinHandle<T> {
    fn drop(&mut self) {
        instrument!(compio_log::Level::TRACE, "JoinHandle::drop");

        if let Some(task) = self.task.as_ref() {
            task.cancel(true);
        }
    }
}
