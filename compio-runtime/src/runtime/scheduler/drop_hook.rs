use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use pin_project_lite::pin_project;

/// Calls a function when dropped.
struct Defer<F: FnMut()>(F);

impl<F: FnMut()> Drop for Defer<F> {
    fn drop(&mut self) {
        (self.0)();
    }
}

pin_project! {
    /// A future wrapper that runs a hook when dropped.
    pub(crate) struct DropHook<Fut, Hook: FnMut()> {
        #[pin]
        future: Fut,
        _hook: Defer<Hook>,
    }
}

impl<Fut, Hook: FnMut()> DropHook<Fut, Hook> {
    /// Creates a new [`DropHook`].
    pub(crate) fn new(future: Fut, hook: Hook) -> Self {
        Self {
            future,
            _hook: Defer(hook),
        }
    }
}

impl<Fut: Future, Hook: FnMut()> Future for DropHook<Fut, Hook> {
    type Output = Fut::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project().future.poll(cx)
    }
}
