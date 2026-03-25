use std::{
    pin::Pin,
    task::{Context, ContextBuilder, Poll},
};

use futures_util::FutureExt;
use pin_project_lite::pin_project;
use synchrony::unsync::event::EventListener;

use crate::{CancelToken, future::Ext};

pin_project! {
    /// A future with a [`CancelToken`] attached to it.
    ///
    /// Created with [`FutureExt::with_cancel`].
    ///
    /// When the cancel token is triggered, this future will still be
    /// polled until completion, only compio operations that registered its [`Key`]
    /// to the cancel token will be cancelled. If you want a future that completes
    /// with an error immediately when the cancel token is triggered, see [`WithCancelFailFast`].
    ///
    /// [`Key`]: compio_driver::Key
    /// [`FutureExt::with_cancel`]: crate::future::FutureExt::with_cancel
    pub struct WithCancel<F: ?Sized> {
        cancel: CancelToken,
        #[pin]
        future: F,
    }
}

pin_project! {
    /// A fail-fast future with a [`CancelToken`] attached to it.
    ///
    /// Created with [`WithCancel::fail_fast`].
    ///
    /// Similar to [`WithCancel`], with the difference that when the
    /// cancel token is triggered, this will also be notified and complete
    /// with an error without further polling the inner future.
    pub struct WithCancelFailFast<F: ?Sized> {
        listen: EventListener,
        #[pin]
        future: WithCancel<F>,
    }
}

/// An [`std::error::Error`] indicating that a future was cancelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Cancelled;

impl<F: ?Sized> WithCancel<F> {
    /// Create a new [`WithCancel`] future.
    pub fn new(future: F, cancel: CancelToken) -> Self
    where
        F: Sized,
    {
        Self { cancel, future }
    }
}

impl<F> WithCancel<F> {
    /// Convert to a fail-fast version.
    ///
    /// When the cancel token is triggered, the future will be notified and
    /// complete with an error without further polling the inner future.
    pub fn fail_fast(self) -> WithCancelFailFast<F> {
        let listen = self.cancel.listen();

        WithCancelFailFast {
            listen,
            future: self,
        }
    }
}

impl<F> WithCancelFailFast<F> {
    /// Convert to a fail-slow version.
    ///
    /// See [`WithCancel`] for details.
    pub fn fail_slow(self) -> WithCancel<F> {
        self.future
    }
}

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Cancelled")
    }
}

impl std::error::Error for Cancelled {}

impl<F: ?Sized> Future for WithCancel<F>
where
    F: Future,
{
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        if let Some(ext) = cx.ext().downcast_mut::<Ext>() {
            ext.set_cancel(this.cancel);
            this.future.poll(cx)
        } else {
            let mut ex = Ext::new().with_cancel(this.cancel);
            let mut cx = ContextBuilder::from(cx).ext(&mut ex).build();
            this.future.poll(&mut cx)
        }
    }
}

impl<F: ?Sized> Future for WithCancelFailFast<F>
where
    F: Future,
{
    type Output = Result<F::Output, Cancelled>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        if this.listen.poll_unpin(cx).is_ready() {
            return Poll::Ready(Err(Cancelled));
        }

        this.future.poll_unpin(cx).map(Ok)
    }
}
