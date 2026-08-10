//! Typed mailboxes, calls, brokers, and delivery errors.

mod call;
mod error;
mod name;
mod receiver;

use std::{
    any::Any,
    fmt,
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub(crate) use call::call_with;
#[doc(inline)]
pub use call::{Call, Reply};
#[doc(inline)]
pub use error::{CallError, DeliverError};
use flume::{Sender, TrySendError};
pub(crate) use name::Name;
pub(crate) use receiver::{MailboxEvent, Receiver, make_mailbox};

use crate::{Actor, Handler, Message, actor::Delivering};

/// Default number of messages reserved for each mailbox.
pub const DEFAULT_MAILBOX_CAPACITY: NonZeroUsize = NonZeroUsize::new(64).unwrap();

struct MailboxInner<A: Actor> {
    name: Option<Name>,
    messages: Sender<Delivering<A>>,
    stop: Sender<()>,
    stopping: AtomicBool,
    capacity: NonZeroUsize,
}

impl<A: Actor> MailboxInner<A> {
    fn send<M>(&self, message: M) -> Result<(), DeliverError<M>>
    where
        A: Handler<M>,
        M: Message,
    {
        if self.is_closed() {
            return Err(DeliverError::Closed(message));
        }

        self.messages
            .try_send(Delivering::<A>::from_msg(message))
            .map_err(|error| match error {
                TrySendError::Full(message) => DeliverError::Full(message.recover::<M>()),
                TrySendError::Disconnected(message) => DeliverError::Closed(message.recover::<M>()),
            })
    }

    fn stop(&self) -> bool {
        if self.stopping.swap(true, Ordering::AcqRel) {
            return false;
        }
        self.stop.try_send(()).is_ok()
    }

    fn begin_stop(&self) {
        self.stopping.store(true, Ordering::Release);
    }

    fn is_closed(&self) -> bool {
        self.stopping.load(Ordering::Acquire)
            || self.messages.is_disconnected()
            || self.stop.is_disconnected()
    }
}

/// A typed reference to an actor in a cluster.
pub struct Mailbox<A: Actor> {
    inner: Arc<MailboxInner<A>>,
}

impl<A: Actor> Mailbox<A> {
    pub(crate) fn begin_stop(&self) {
        self.inner.begin_stop();
    }

    /// Returns the actor's registered name.
    pub fn name(&self) -> Option<&str> {
        self.inner.name.as_ref().map(Name::as_str)
    }

    /// Enqueues a message handled by this actor without waiting.
    pub fn send<M>(&self, message: M) -> Result<(), DeliverError<M>>
    where
        A: Handler<M>,
        M: Message,
    {
        self.inner.send(message)
    }

    /// Creates a send-only capability for one message type.
    pub fn broker<M>(&self) -> Broker<M>
    where
        A: Handler<M>,
        M: Message,
    {
        let inner: Arc<dyn BrokerSink<M>> = self.inner.clone();
        Broker { inner }
    }

    /// Requests a graceful stop, returning whether this call requested it.
    pub fn stop(&self) -> bool {
        self.inner.stop()
    }

    /// Returns whether the mailbox rejects new messages.
    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }

    /// Returns the fixed mailbox capacity.
    pub fn capacity(&self) -> NonZeroUsize {
        self.inner.capacity
    }
}

impl<A: Actor> Clone for Mailbox<A> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<A: Actor> fmt::Debug for Mailbox<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Mailbox")
            .field("name", &self.name())
            .field("capacity", &self.inner.capacity)
            .field("queued", &self.inner.messages.len())
            .field("closed", &self.is_closed())
            .finish()
    }
}

trait BrokerSink<M: Message>: Send + Sync {
    fn name(&self) -> Option<&str>;
    fn send(&self, message: M) -> Result<(), DeliverError<M>>;
}

impl<A, M> BrokerSink<M> for MailboxInner<A>
where
    A: Handler<M>,
    M: Message,
{
    fn name(&self) -> Option<&str> {
        self.name.as_ref().map(Name::as_str)
    }

    fn send(&self, message: M) -> Result<(), DeliverError<M>> {
        MailboxInner::send(self, message)
    }
}

/// A send-only capability for messages of type `M`.
pub struct Broker<M: Message> {
    inner: Arc<dyn BrokerSink<M>>,
}

impl<M: Message> Broker<M> {
    /// Returns the actor's registered name.
    pub fn name(&self) -> Option<&str> {
        self.inner.name()
    }

    /// Enqueues a message without waiting.
    pub fn send(&self, message: M) -> Result<(), DeliverError<M>> {
        self.inner.send(message)
    }
}

impl<M: Message> Clone for Broker<M> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<M: Message> fmt::Debug for Broker<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Broker")
            .field("name", &self.name())
            .finish_non_exhaustive()
    }
}

pub(crate) type ErasedMailbox = Arc<dyn Any + Send + Sync>;

impl<A: Actor> Mailbox<A> {
    pub(crate) fn erase(&self) -> ErasedMailbox {
        self.inner.clone()
    }

    pub(crate) fn from_erased(inner: ErasedMailbox) -> Option<Self> {
        Arc::downcast::<MailboxInner<A>>(inner)
            .ok()
            .map(|inner| Self { inner })
    }
}
