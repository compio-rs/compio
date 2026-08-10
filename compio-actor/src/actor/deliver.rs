use std::{any::Any, future::Future, pin::Pin};

use super::{Actor, ActorExit, Handler, Message};
use crate::{
    Mailbox,
    mailbox::{MailboxEvent, Receiver},
    supervisor::Supervision,
};

type DeliveryFuture<'a, E> = Pin<Box<dyn Future<Output = Result<(), E>> + 'a>>;

trait Deliverable<A: Actor>: Send {
    fn deliver_to<'a>(
        self: Box<Self>,
        actor: &'a A,
        myself: &'a Mailbox<A>,
        state: &'a mut A::State,
    ) -> DeliveryFuture<'a, A::Error>;

    fn into_any(self: Box<Self>) -> Box<dyn Any + Send>;
}

struct Envelope<M: Message>(M);

impl<A, M> Deliverable<A> for Envelope<M>
where
    A: Handler<M>,
    M: Message,
{
    fn deliver_to<'a>(
        self: Box<Self>,
        actor: &'a A,
        myself: &'a Mailbox<A>,
        state: &'a mut A::State,
    ) -> DeliveryFuture<'a, A::Error> {
        let Self(message) = *self;
        Box::pin(async move { Handler::<M>::handle(actor, myself, message, state).await })
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any + Send> {
        self
    }
}

pub(crate) struct Delivering<A: Actor>(Box<dyn Deliverable<A> + Send>);

impl<A: Actor> Delivering<A> {
    pub(crate) fn from_msg<M>(message: M) -> Self
    where
        A: Handler<M>,
        M: Message,
    {
        Self(Box::new(Envelope(message)))
    }

    pub(crate) fn recover<M>(self) -> M
    where
        A: Handler<M>,
        M: Message,
    {
        let Envelope(message) = *self
            .0
            .into_any()
            .downcast::<Envelope<M>>()
            .expect("message envelope type changed before delivery");
        message
    }

    pub(crate) fn deliver_to<'a>(
        self,
        actor: &'a A,
        myself: &'a Mailbox<A>,
        state: &'a mut A::State,
    ) -> DeliveryFuture<'a, A::Error> {
        self.0.deliver_to(actor, myself, state)
    }
}

pub(crate) async fn run<A: Actor>(
    actor: A,
    myself: Mailbox<A>,
    receiver: Receiver<A>,
    mut state: A::State,
    supervision: Option<&Supervision<A>>,
) -> ActorExit<A::Error> {
    let exit = match actor.post_start(&myself, &mut state).await {
        Ok(()) => {
            if let Some(supervision) = supervision {
                supervision.started(&myself);
            }
            loop {
                match receiver.recv().await {
                    MailboxEvent::Message(message) => {
                        if let Err(error) = message.deliver_to(&actor, &myself, &mut state).await {
                            break ActorExit::Failed(error);
                        }
                    }
                    MailboxEvent::Stop => break ActorExit::Stopped,
                }
            }
        }
        Err(error) => ActorExit::Failed(error),
    };

    finish(&actor, &myself, receiver, &mut state, exit).await
}

pub(crate) async fn finish<A: Actor>(
    actor: &A,
    myself: &Mailbox<A>,
    receiver: Receiver<A>,
    state: &mut A::State,
    mut exit: ActorExit<A::Error>,
) -> ActorExit<A::Error> {
    myself.begin_stop();
    if let Err(error) = actor.pre_stop(myself, state).await
        && matches!(exit, ActorExit::Stopped)
    {
        exit = ActorExit::Failed(error);
    }

    drop(receiver);
    if let Err(error) = actor.post_stop(myself, state).await
        && matches!(exit, ActorExit::Stopped)
    {
        exit = ActorExit::Failed(error);
    }
    exit
}
