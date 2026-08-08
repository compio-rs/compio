//! Actor cluster, registry, and spawn configuration.

mod current;
mod registry;
mod spawn;

use std::{
    borrow::Cow,
    io,
    sync::{Arc, Mutex},
};

use compio_dispatcher::Dispatcher;
use registry::Registry;
#[doc(inline)]
pub use spawn::{Spawn, SpawnError, SpawnFuture, SpawnResult};

use crate::{Actor, Mailbox};

/// A set of Compio workers on which actors are placed.
#[derive(Clone)]
pub struct Cluster {
    inner: Arc<ClusterInner>,
}

struct ClusterInner {
    dispatcher: Mutex<Option<Dispatcher>>,
    registry: Registry,
}

impl Cluster {
    /// Creates a cluster using the dispatcher's defaults.
    pub fn new() -> io::Result<Self> {
        Dispatcher::new().map(Self::from_dispatcher)
    }

    /// Creates a cluster from a configured dispatcher.
    pub fn from_dispatcher(dispatcher: Dispatcher) -> Self {
        Self {
            inner: Arc::new(ClusterInner {
                dispatcher: Mutex::new(Some(dispatcher)),
                registry: Registry::default(),
            }),
        }
    }

    /// Configures an actor spawn operation.
    ///
    /// The return type is a configurable future implementing [`IntoFuture`].
    /// See [`Spawn`] for detail.
    pub fn spawn<A, F>(&self, factory: F, arguments: A::Arguments) -> Spawn<'_, A, F>
    where
        A: Actor,
        F: FnOnce() -> A + Send + 'static,
    {
        Spawn::new(self, factory, arguments)
    }

    /// Looks up a named actor of type `A`.
    pub fn lookup<A, N>(&self, name: N) -> Option<Mailbox<A>>
    where
        A: Actor,
        N: Into<Cow<'static, str>>,
    {
        let name = name.into();
        self.inner.registry.get(&name)
    }

    /// Stops the dispatcher workers and waits for their threads to exit.
    pub async fn join(self) -> io::Result<()> {
        let dispatcher = self.inner.dispatcher.lock().unwrap().take();
        match dispatcher {
            Some(dispatcher) => dispatcher.join().await,
            None => Ok(()),
        }
    }
}
