use std::{
    num::NonZeroUsize,
    sync::{Arc, atomic::AtomicBool},
};

use flume::Receiver as FlumeReceiver;
use futures_util::{FutureExt, pin_mut, select_biased};

use super::{Mailbox, MailboxInner, Name};
use crate::{Actor, actor::Delivering};

pub(crate) struct Receiver<A: Actor> {
    messages: FlumeReceiver<Delivering<A>>,
    stop: FlumeReceiver<()>,
}

impl<A: Actor> Receiver<A> {
    pub(crate) async fn recv(&self) -> MailboxEvent<A> {
        let stop = self.stop.recv_async().fuse();
        let message = self.messages.recv_async().fuse();
        pin_mut!(stop, message);

        select_biased! {
            _ = stop => MailboxEvent::Stop,
            message = message => match message {
                Ok(message) => MailboxEvent::Message(message),
                Err(_) => MailboxEvent::Stop,
            },
        }
    }
}

pub(crate) enum MailboxEvent<A: Actor> {
    Message(Delivering<A>),
    Stop,
}

pub(crate) fn make_mailbox<A: Actor>(
    name: Option<Name>,
    capacity: NonZeroUsize,
) -> (Mailbox<A>, Receiver<A>) {
    let (message_tx, message_rx) = flume::bounded(capacity.get());
    let (stop_tx, stop_rx) = flume::bounded(1);
    let inner = Arc::new(MailboxInner {
        name,
        messages: message_tx,
        stop: stop_tx,
        stopping: AtomicBool::new(false),
        capacity,
    });

    (
        Mailbox { inner },
        Receiver {
            messages: message_rx,
            stop: stop_rx,
        },
    )
}
