//! Actor lifecycle notifications.
//!
//! A supervisor is an ordinary actor that handles [`SupervisionEvent`] for a
//! child type. Attach it while spawning the child:
//!
//! ```rust
//! use std::{convert::Infallible, marker::PhantomData};
//!
//! use compio_actor::{Actor, Cluster, Handler, Mailbox, supervisor::SupervisionEvent};
//!
//! struct Child;
//! struct Supervisor<C>(PhantomData<fn() -> C>);
//!
//! impl<C> Supervisor<C> {
//!     fn new() -> Self {
//!         Self(PhantomData)
//!     }
//! }
//!
//! # impl Actor for Child {
//! #     type Arguments = ();
//! #     type Error = Infallible;
//! #     type State = ();
//! #
//! #     async fn pre_start(
//! #         &self,
//! #         _myself: &Mailbox<Self>,
//! #         (): Self::Arguments,
//! #     ) -> Result<Self::State, Self::Error> {
//! #         Ok(())
//! #     }
//! # }
//! #
//! # impl<C: Actor> Actor for Supervisor<C> {
//! #     type Arguments = ();
//! #     type Error = Infallible;
//! #     type State = ();
//! #
//! #     async fn pre_start(
//! #         &self,
//! #         _myself: &Mailbox<Self>,
//! #         (): Self::Arguments,
//! #     ) -> Result<Self::State, Self::Error> {
//! #         Ok(())
//! #     }
//! # }
//!
//! impl<C: Actor> Handler<SupervisionEvent<C>> for Supervisor<C> {
//!     async fn handle(
//!         &self,
//!         _myself: &Mailbox<Self>,
//!         event: SupervisionEvent<C>,
//!         _state: &mut Self::State,
//!     ) -> Result<(), Self::Error> {
//!         match event {
//!             SupervisionEvent::ActorStarted(child) => {
//!                 child.stop();
//!             }
//!             SupervisionEvent::ActorTerminated(_) => {}
//!             SupervisionEvent::ActorFailed(_) => {}
//!         }
//!         Ok(())
//!     }
//! }
//!
//! # async fn example() -> std::io::Result<()> {
//! let cluster = Cluster::new()?;
//! let (supervisor, _supervisor_handle) =
//!     cluster.spawn(Supervisor::<Child>::new, ()).await.unwrap();
//! let (_child, _child_handle) = cluster
//!     .spawn(|| Child, ())
//!     .with_supervisor(&supervisor)
//!     .await
//!     .unwrap();
//! # Ok(())
//! # }
//! ```

use std::fmt;

use crate::{Actor, Broker, Handler, Mailbox};

/// A lifecycle event emitted by a supervised actor.
pub enum SupervisionEvent<A: Actor> {
    /// The actor completed its startup hooks.
    ActorStarted(Mailbox<A>),
    /// The actor stopped normally.
    ActorTerminated(Mailbox<A>),
    /// The actor exited after a lifecycle or handler error.
    ActorFailed(Mailbox<A>),
}

impl<A: Actor> SupervisionEvent<A> {
    /// Returns the actor that emitted this event.
    pub fn actor(&self) -> &Mailbox<A> {
        match self {
            Self::ActorStarted(actor) | Self::ActorTerminated(actor) | Self::ActorFailed(actor) => {
                actor
            }
        }
    }
}

impl<A: Actor> fmt::Debug for SupervisionEvent<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ActorStarted(actor) => f.debug_tuple("ActorStarted").field(actor).finish(),
            Self::ActorTerminated(actor) => f.debug_tuple("ActorTerminated").field(actor).finish(),
            Self::ActorFailed(actor) => f.debug_tuple("ActorFailed").field(actor).finish(),
        }
    }
}

pub(crate) struct Supervision<A: Actor> {
    broker: Broker<SupervisionEvent<A>>,
}

impl<A: Actor> Supervision<A> {
    pub(crate) fn new<S>(supervisor: &Mailbox<S>) -> Self
    where
        S: Handler<SupervisionEvent<A>>,
    {
        Self {
            broker: supervisor.broker(),
        }
    }

    pub(crate) fn started(&self, actor: &Mailbox<A>) {
        self.broker
            .send(SupervisionEvent::ActorStarted(actor.clone()))
            .ok();
    }

    pub(crate) fn terminated(&self, actor: &Mailbox<A>) {
        self.broker
            .send(SupervisionEvent::ActorTerminated(actor.clone()))
            .ok();
    }

    pub(crate) fn failed(&self, actor: &Mailbox<A>) {
        self.broker
            .send(SupervisionEvent::ActorFailed(actor.clone()))
            .ok();
    }
}
