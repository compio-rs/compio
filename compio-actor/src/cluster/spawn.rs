use std::{
    borrow::Cow,
    error::Error,
    fmt,
    future::{Future, IntoFuture},
    num::NonZeroUsize,
    pin::Pin,
    task::{Context, Poll, ready},
};

use futures_channel::oneshot;
use futures_util::FutureExt;

use super::Cluster;
use crate::{
    Actor, Handler, Mailbox,
    actor::{ActorExit, ActorHandle, finish, run},
    mailbox::{DEFAULT_MAILBOX_CAPACITY, Name, make_mailbox},
    supervisor::{Supervision, SupervisionEvent},
};

/// The result returned when an actor starts successfully.
pub type SpawnResult<A> =
    Result<(Mailbox<A>, ActorHandle<<A as Actor>::Error>), SpawnError<<A as Actor>::Error>>;

/// An error encountered while starting an actor.
#[derive(Debug, PartialEq, Eq)]
pub enum SpawnError<E: Send + 'static> {
    /// The cluster is no longer accepting actors.
    Unavailable,
    /// Another actor is registered under this name.
    NameTaken(Cow<'static, str>),
    /// The actor's startup hook failed.
    Start(E),
    /// The worker stopped before startup completed.
    WorkerStopped,
}

impl<E: Send + 'static> fmt::Display for SpawnError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => f.write_str("actor cluster is unavailable"),
            Self::NameTaken(name) => write!(f, "actor name {name:?} is already registered"),
            Self::Start(_) => f.write_str("actor startup failed"),
            Self::WorkerStopped => f.write_str("actor worker stopped during startup"),
        }
    }
}

impl<E: Error + Send + 'static> Error for SpawnError<E> {}

/// A configurable actor spawn operation.
///
/// Returned by [`Cluster::spawn`].
#[must_use = "actors are not spawned until this builder is awaited"]
pub struct Spawn<'a, A, F>
where
    A: Actor,
    F: FnOnce() -> A + Send + 'static,
{
    cluster: &'a Cluster,
    factory: F,
    arguments: A::Arguments,
    name: Option<Cow<'static, str>>,
    capacity: NonZeroUsize,
    supervisor: Option<Supervision<A>>,
}

impl<'a, A, F> Spawn<'a, A, F>
where
    A: Actor,
    F: FnOnce() -> A + Send + 'static,
{
    pub(super) fn new(cluster: &'a Cluster, factory: F, arguments: A::Arguments) -> Self {
        Self {
            cluster,
            factory,
            arguments,
            name: None,
            capacity: DEFAULT_MAILBOX_CAPACITY,
            supervisor: None,
        }
    }

    /// Registers the actor under `name` after startup succeeds.
    pub fn with_name(mut self, name: impl Into<Cow<'static, str>>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Sets the actor's bounded mailbox capacity.
    pub fn with_capacity(mut self, capacity: NonZeroUsize) -> Self {
        self.capacity = capacity;
        self
    }

    /// Sends actor lifecycle events to `supervisor`.
    pub fn with_supervisor<S>(mut self, supervisor: &Mailbox<S>) -> Self
    where
        S: Handler<SupervisionEvent<A>>,
    {
        self.supervisor = Some(Supervision::new(supervisor));
        self
    }
}

impl<A, F> IntoFuture for Spawn<'_, A, F>
where
    A: Actor,
    F: FnOnce() -> A + Send + 'static,
{
    type IntoFuture = SpawnFuture<A>;
    type Output = SpawnResult<A>;

    fn into_future(self) -> Self::IntoFuture {
        self.cluster.start(
            self.factory,
            self.arguments,
            self.name,
            self.capacity,
            self.supervisor,
        )
    }
}

/// The future produced by [`Spawn`].
pub enum SpawnFuture<A: Actor> {
    #[doc(hidden)]
    Ready(Option<SpawnResult<A>>),
    #[doc(hidden)]
    Pending {
        mailbox: Mailbox<A>,
        result: oneshot::Receiver<Result<ActorExit<A::Error>, ()>>,
        started: oneshot::Receiver<Result<(), A::Error>>,
    },
}

impl<A: Actor> SpawnFuture<A> {
    fn ready(result: SpawnResult<A>) -> Self {
        Self::Ready(Some(result))
    }

    fn pending(
        mailbox: Mailbox<A>,
        result: oneshot::Receiver<Result<ActorExit<A::Error>, ()>>,
        started: oneshot::Receiver<Result<(), A::Error>>,
    ) -> Self {
        Self::Pending {
            mailbox,
            result,
            started,
        }
    }
}

impl<A: Actor> Future for SpawnFuture<A> {
    type Output = SpawnResult<A>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let started = match this {
            Self::Ready(result) => {
                return Poll::Ready(result.take().expect("spawn future polled after completion"));
            }
            Self::Pending { started, .. } => ready!(started.poll_unpin(cx)),
        };
        let (mailbox, result) = match std::mem::replace(this, Self::Ready(None)) {
            Self::Pending {
                mailbox, result, ..
            } => (mailbox, result),
            Self::Ready(_) => unreachable!(),
        };
        Poll::Ready(match started {
            Ok(Ok(())) => Ok((mailbox, ActorHandle { result })),
            Ok(Err(error)) => Err(SpawnError::Start(error)),
            Err(_) => Err(SpawnError::WorkerStopped),
        })
    }
}

impl<A: Actor> Unpin for SpawnFuture<A> {}

impl Cluster {
    fn start<A, F>(
        &self,
        factory: F,
        arguments: A::Arguments,
        name: Option<Cow<'static, str>>,
        capacity: NonZeroUsize,
        supervisor: Option<Supervision<A>>,
    ) -> SpawnFuture<A>
    where
        A: Actor,
        F: FnOnce() -> A + Send + 'static,
    {
        let (name, reg) = match name {
            Some(name) => {
                let name = Name::from(name);
                match self.inner.registry.reserve(name.clone()) {
                    Ok(registration) => (Some(name), Some(registration)),
                    Err(name) => {
                        return SpawnFuture::ready(Err(SpawnError::NameTaken(name.into_cow())));
                    }
                }
            }
            None => (None, None),
        };

        let (mailbox, receiver) = make_mailbox::<A>(name, capacity);
        let actor_ref = mailbox.clone();
        let (started_tx, started_rx) = oneshot::channel();
        let cluster = self.clone();
        let result = {
            let dispatcher = self.inner.dispatcher.lock().unwrap();
            let Some(dispatcher) = dispatcher.as_ref() else {
                return SpawnFuture::ready(Err(SpawnError::Unavailable));
            };
            dispatcher.dispatch(move || {
                cluster.drive(async move {
                    let mut reg = reg;
                    let actor = factory();
                    let mut state = match actor.pre_start(&actor_ref, arguments).await {
                        Ok(state) => state,
                        Err(error) => {
                            reg.take();
                            started_tx.send(Err(error)).ok();
                            return Err(());
                        }
                    };
                    if let Some(registration) = &reg {
                        registration.activate(&actor_ref);
                    }

                    if started_tx.send(Ok(())).is_err() {
                        let exit =
                            finish(&actor, &actor_ref, receiver, &mut state, ActorExit::Stopped)
                                .await;
                        return Ok(exit);
                    }

                    let exit = run(
                        actor,
                        actor_ref.clone(),
                        receiver,
                        state,
                        supervisor.as_ref(),
                    )
                    .await;
                    drop(reg);
                    if let Some(supervisor) = supervisor {
                        match &exit {
                            ActorExit::Stopped => supervisor.terminated(&actor_ref),
                            ActorExit::Failed(_) => supervisor.failed(&actor_ref),
                        }
                    }
                    Ok(exit)
                })
            })
        };
        let result = match result {
            Ok(result) => result,
            Err(_) => return SpawnFuture::ready(Err(SpawnError::Unavailable)),
        };

        SpawnFuture::pending(mailbox, result, started_rx)
    }
}
