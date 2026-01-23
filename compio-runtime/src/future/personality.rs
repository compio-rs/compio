use std::{
    pin::Pin,
    task::{Context, ContextBuilder, Poll},
};

use pin_project_lite::pin_project;

use crate::future::Ext;

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
        if let Some(ext) = cx.ext().downcast_mut::<Ext>() {
            ext.set_personality(*this.personality);
            this.future.poll(cx)
        } else {
            let mut ex = Ext::new().with_personality(*this.personality);
            let mut cx = ContextBuilder::from(cx).ext(&mut ex).build();
            this.future.poll(&mut cx)
        }
    }
}
