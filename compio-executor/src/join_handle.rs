use std::{
    marker::PhantomData,
    mem::ManuallyDrop,
    panic::resume_unwind,
    pin::Pin,
    ptr,
    task::{Context, Poll},
};

use crate::{Panic, task::Task};

/// A handle that awaits the result of a task.
///
/// Dropping a [`JoinHandle`] will cancel the task. To run the task in the
/// background, use [`JoinHandle::detach`].
#[must_use = "Drop `JoinHandle` will cancel the task. Use `detach` to run it in background."]
#[derive(Debug)]
#[repr(transparent)]
pub struct JoinHandle<T> {
    task: Task,
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
            task,
            _marker: PhantomData,
        }
    }

    /// Cancel the task.
    pub async fn cancel(self) -> Option<T> {
        self.task.cancel();
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

impl<T> Future for JoinHandle<T> {
    type Output = Result<T, JoinError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { self.task.poll(cx) }.map(|res| match res {
            Some(Ok(res)) => Ok(res),
            Some(Err(e)) => Err(JoinError::Panicked(e)),
            None => Err(JoinError::Cancelled),
        })
    }
}

impl<T> Drop for JoinHandle<T> {
    fn drop(&mut self) {
        self.task.cancel();
    }
}
