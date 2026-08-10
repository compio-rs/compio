use std::fmt;

use futures_channel::oneshot;

use super::{Broker, CallError, DeliverError, Mailbox};
use crate::{Actor, Handler, Message};

impl<A: Actor> Mailbox<A> {
    /// Sends a request and waits for the actor's reply.
    pub async fn call<M, R>(&self, message: M) -> Result<R, CallError<M>>
    where
        A: Handler<Call<M, R>>,
        M: Message,
        R: Message,
    {
        call_with(message, |call| self.inner.send(call)).await
    }
}

impl<M: Message, R: Message> Broker<Call<M, R>> {
    /// Sends a request and waits for the actor's reply.
    pub async fn call(&self, message: M) -> Result<R, CallError<M>> {
        call_with(message, |call| self.send(call)).await
    }
}

/// A request together with the channel used to answer it.
pub struct Call<M: Message, R: Message> {
    message: M,
    reply: Reply<R>,
}

impl<M: Message, R: Message> Call<M, R> {
    fn new(message: M, sender: oneshot::Sender<R>) -> Self {
        Self {
            message,
            reply: Reply(sender),
        }
    }

    /// Get the request message.
    pub fn message(&self) -> &M {
        &self.message
    }

    /// Answers the call, returning the response if the caller stopped waiting.
    pub fn reply(self, response: R) -> Result<(), R> {
        self.reply.reply(response)
    }

    /// Splits the owned request from its reply capability.
    pub fn into_parts(self) -> (M, Reply<R>) {
        (self.message, self.reply)
    }

    pub(super) fn into_message(self) -> M {
        self.message
    }
}

impl<M: Message + fmt::Debug, R: Message> fmt::Debug for Call<M, R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Call")
            .field("message", &self.message)
            .finish_non_exhaustive()
    }
}

/// A port used to send a reply to a [`Call`].
pub struct Reply<R: Message>(oneshot::Sender<R>);

impl<R: Message> Reply<R> {
    /// Answers the call, returning the response if the caller stopped waiting.
    pub fn reply(self, response: R) -> Result<(), R> {
        self.0.send(response)
    }
}

impl<R: Message> fmt::Debug for Reply<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Reply").finish_non_exhaustive()
    }
}

pub(crate) async fn call_with<M, R>(
    message: M,
    send: impl FnOnce(Call<M, R>) -> Result<(), DeliverError<Call<M, R>>>,
) -> Result<R, CallError<M>>
where
    M: Message,
    R: Message,
{
    let (sender, receiver) = oneshot::channel();
    send(Call::new(message, sender)).map_err(CallError::from_deliver)?;
    receiver.await.map_err(|_| CallError::NoReply)
}
