use std::{
    error::Error,
    fmt,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use futures_channel::oneshot;

/// The observed result of an actor task.
#[derive(Debug, PartialEq, Eq)]
pub enum ActorExit<E: Send + 'static> {
    /// The actor stopped normally.
    Stopped,
    /// A lifecycle method failed.
    Failed(E),
}

/// The worker stopped before reporting an actor's exit.
#[derive(Debug, PartialEq, Eq)]
pub struct ActorHandleError;

impl fmt::Display for ActorHandleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("actor worker stopped before reporting an exit")
    }
}

impl Error for ActorHandleError {}

/// A handle for observing an actor running in the cluster.
pub struct ActorHandle<E: Send + 'static> {
    pub(crate) result: oneshot::Receiver<Result<ActorExit<E>, ()>>,
}

impl<E: Send + 'static> Future for ActorHandle<E> {
    type Output = Result<ActorExit<E>, ActorHandleError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.get_mut().result)
            .poll(cx)
            .map(|result| result.ok().and_then(Result::ok).ok_or(ActorHandleError))
    }
}
