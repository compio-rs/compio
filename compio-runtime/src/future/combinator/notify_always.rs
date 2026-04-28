use std::{
    pin::Pin,
    task::{Context, Poll},
};

use pin_project_lite::pin_project;

use crate::{
    future::Ext,
    waker::{ExtWaker, with_ext},
};

pin_project! {
    /// A future combinator that will always be notified when the inner future is
    /// ready, even if the inner future is already ready when this combinator is
    /// created.
    ///
    /// Created with [`FutureExt::with_notify_always`].
    ///
    /// [`FutureExt::with_notify_always`]: crate::future::FutureExt::with_notify_always
    pub struct WithNotifyAlways<F: ?Sized> {
        notify_always: bool,
        #[pin]
        future: F,
    }
}

impl<F: ?Sized> WithNotifyAlways<F> {
    /// Create a new [`WithNotifyAlways`] future.
    pub fn new(future: F, notify: bool) -> Self
    where
        F: Sized,
    {
        Self {
            notify_always: notify,
            future,
        }
    }
}

impl<F: Future + ?Sized> Future for WithNotifyAlways<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        with_ext(cx.waker(), |waker, ext: &Ext| {
            ext.set_notify_always(*this.notify_always);
            ExtWaker::new(waker, ext).poll(this.future)
        })
    }
}
