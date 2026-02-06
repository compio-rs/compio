use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    future::poll_fn,
    mem,
    ops::DerefMut,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use compio_driver::{Cancel, Key, OpCode};
use futures_util::{FutureExt, ready};
use synchrony::unsync::event::{Event, EventListener};

use crate::Runtime;

#[derive(Debug)]
struct Inner {
    tokens: RefCell<HashSet<Cancel>>,
    is_cancelled: Cell<bool>,
    runtime: Runtime,
    notify: Event,
}

/// A token that can be used to cancel multiple operations at once.
///
/// When [`CancelToken::cancel`] is called, all operations that have been
/// registered with this token will be cancelled.
///
/// It is also possible to use [`CancelToken::wait`] to wait until the token is
/// cancelled, which can be useful for implementing timeouts or other
/// cancellation-based logic.
///
/// To associate a future with this cancel token, use the `with_cancel`
/// combinator from the [`FutureExt`] trait, which requires nightly feature
/// `future-combinator`.
#[cfg_attr(
    feature = "future-combinator",
    doc = "\n\n [`FutureExt`]: crate::future::FutureExt"
)]
#[cfg_attr(
    not(feature = "future-combinator"),
    doc = "\n\n [`FutureExt`]: https://docs.rs/compio/latest/compio/runtime/future/trait.FutureExt.html"
)]
#[derive(Clone, Debug)]
pub struct CancelToken(Rc<Inner>);

impl PartialEq for CancelToken {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for CancelToken {}

impl CancelToken {
    /// Create a new cancel token.
    pub fn new() -> Self {
        Self(Rc::new(Inner {
            tokens: RefCell::new(HashSet::new()),
            is_cancelled: Cell::new(false),
            runtime: Runtime::current(),
            notify: Event::new(),
        }))
    }

    pub(crate) fn listen(&self) -> EventListener {
        self.0.notify.listen()
    }

    /// Cancel all operations registered with this token.
    pub fn cancel(self) {
        self.0.notify.notify_all();
        if self.0.is_cancelled.replace(true) {
            return;
        }
        let tokens = mem::take(self.0.tokens.borrow_mut().deref_mut());
        for t in tokens {
            self.0.runtime.cancel_token(t);
        }
    }

    /// Check if this token has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.0.is_cancelled.get()
    }

    /// Register an operation with this token.
    ///
    /// If the token has already been cancelled, the operation will be cancelled
    /// immediately. Usually this method should not be used directly, but rather
    /// through the `with_cancel` combinator, which requires nightly feature
    /// `future-combinator`.
    ///
    /// Multiple registrations of the same key does nothing, and the key will
    /// only be cancelled once.
    pub fn register<T: OpCode>(&self, key: &Key<T>) {
        if self.0.is_cancelled.get() {
            self.0.runtime.cancel(key.clone());
        } else {
            let token = self.0.runtime.register_cancel(key);
            self.0.tokens.borrow_mut().insert(token);
        }
    }

    /// Wait until this token is cancelled.
    pub fn wait(self) -> WaitFuture {
        WaitFuture::new(self)
    }

    /// Try to get the current cancel token associated with the future.
    ///
    /// This is done by checking if the current context has a cancel token
    /// associated with it. This will only work with nightly feature
    /// `future-combinator` turned on; otherwise, `None` is always returned.
    pub async fn current() -> Option<Self> {
        #[cfg(not(feature = "future-combinator"))]
        return None;

        #[cfg(feature = "future-combinator")]
        poll_fn(|cx| {
            use crate::runtime::ContextExt;
            Poll::Ready(cx.as_cancel().cloned())
        })
        .await
    }
}

impl Default for CancelToken {
    fn default() -> Self {
        Self::new()
    }
}

/// Future returned by [`CancelToken::wait`].
pub struct WaitFuture {
    listen: EventListener,
    token: CancelToken,
}

impl WaitFuture {
    fn new(token: CancelToken) -> WaitFuture {
        WaitFuture {
            listen: token.0.notify.listen(),
            token,
        }
    }
}

impl Future for WaitFuture {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<()> {
        loop {
            if self.token.is_cancelled() {
                return Poll::Ready(());
            } else {
                ready!(self.listen.poll_unpin(cx))
            }
        }
    }
}
