//! Actor definitions, lifecycle results, and handles.

mod deliver;
mod handle;

pub(crate) use deliver::{Delivering, finish, run};
#[doc(inline)]
pub use handle::{ActorExit, ActorHandle, ActorHandleError};

use crate::Mailbox;

/// A message that can cross into an actor cluster.
pub trait Message: Send + 'static {}

impl<T: Send + 'static> Message for T {}

/// A single-threaded actor with serial access to its state.
#[allow(async_fn_in_trait)]
pub trait Actor: Sized + 'static {
    /// State owned by the actor task.
    type State: 'static;
    /// Values moved to the worker to initialize the actor.
    type Arguments: Send + 'static;
    /// Errors reported across the cluster.
    type Error: Send + 'static;

    /// Initializes state on the actor's worker.
    async fn pre_start(
        &self,
        myself: &Mailbox<Self>,
        arguments: Self::Arguments,
    ) -> Result<Self::State, Self::Error>;

    /// Runs inside the actor task before it receives messages.
    async fn post_start(
        &self,
        _myself: &Mailbox<Self>,
        _state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Runs after message processing ends but before the mailbox is dropped.
    async fn pre_stop(
        &self,
        _myself: &Mailbox<Self>,
        _state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Runs after the mailbox has closed.
    async fn post_stop(
        &self,
        _myself: &Mailbox<Self>,
        _state: &mut Self::State,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// Handles messages of type `M` for an actor.
#[allow(async_fn_in_trait)]
pub trait Handler<M: Message>: Actor {
    /// Handles one message at a time.
    async fn handle(
        &self,
        myself: &Mailbox<Self>,
        message: M,
        state: &mut Self::State,
    ) -> Result<(), Self::Error>;
}
