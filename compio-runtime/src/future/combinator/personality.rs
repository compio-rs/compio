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
    /// A future with a personality attached to it.
    pub struct WithPersonality<F: ?Sized> {
        personality: u16,
        #[pin]
        future: F,
    }
}

impl<F> WithPersonality<F> {
    /// Create a new [`WithPersonality`] future.
    pub fn new(future: F, personality: u16) -> Self {
        Self {
            future,
            personality,
        }
    }
}

impl<F: Future + ?Sized> Future for WithPersonality<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        with_ext(cx.waker(), |waker, ext: &Ext| {
            let ext = ext.with_personality(*this.personality);
            ExtWaker::new(waker, &ext).poll(this.future)
        })
    }
}
