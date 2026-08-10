use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use pin_project_lite::pin_project;

use super::Cluster;

scoped_tls::scoped_thread_local!(static CURRENT_CLUSTER: Cluster);

impl Cluster {
    /// Returns the cluster running the current actor.
    ///
    /// # Panics
    ///
    /// Panics when called outside an actor managed by a cluster.
    pub fn current() -> Self {
        Cluster::try_current().expect("not running in an actor cluster")
    }

    /// Try to get the cluster running the current actor.
    pub fn try_current() -> Option<Self> {
        CURRENT_CLUSTER
            .is_set()
            .then(|| CURRENT_CLUSTER.with(Clone::clone))
    }

    /// Drive the future in the scope of `self`.
    pub(super) fn drive<F: Future>(self, future: F) -> Scope<F> {
        Scope {
            cluster: self,
            future,
        }
    }
}

pin_project! {
    pub(super) struct Scope<F> {
        cluster: Cluster,
        #[pin]
        future: F,
    }
}

impl<F: Future> Future for Scope<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        CURRENT_CLUSTER.set(this.cluster, || this.future.poll(cx))
    }
}
