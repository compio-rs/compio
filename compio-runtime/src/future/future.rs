//! Future for submitting operations to the runtime.

use std::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll, Waker},
};

use compio_buf::BufResult;
use compio_driver::{Extra, Key, OpCode, PushEntry};
use futures_util::future::FusedFuture;

use crate::{
    CancelToken, Runtime,
    waker::{get_ext, get_waker},
};

pub(crate) trait ContextExt {
    /// Remove all wrapped [`ExtWaker`] and return the underlying waker.
    ///
    /// This is the same as calling [`Context::waker`] if the waker was never
    /// wrapped.
    fn get_waker(&self) -> &Waker;

    /// Get the cancel token
    fn get_cancel(&mut self) -> Option<&CancelToken>;

    /// Set the ext data associated with the waker to an [`Extra`].
    fn as_extra(&mut self, default: impl FnOnce() -> Extra) -> Option<Extra>;
}

impl ContextExt for Context<'_> {
    fn get_waker(&self) -> &Waker {
        get_waker(self.waker())
    }

    fn get_cancel(&mut self) -> Option<&CancelToken> {
        get_ext(self.waker())?.get_cancel()
    }

    fn as_extra(&mut self, default: impl FnOnce() -> Extra) -> Option<Extra> {
        let ext = get_ext(self.waker())?;
        let mut extra = default();
        ext.set_extra(&mut extra);
        Some(extra)
    }
}

pin_project_lite::pin_project! {
    /// Returned [`Future`] for [`Runtime::submit`].
    ///
    /// When this is dropped and the operation hasn't finished yet, it will try to
    /// cancel the operation.
    ///
    /// By default, this implements `Future<Output = BufResult<usize, T>>`. If
    /// [`Extra`] is needed, call [`.with_extra()`] to get a `Submit<T, Extra>`
    /// which implements `Future<Output = (BufResult<usize, T>, Extra)>`.
    ///
    /// [`.with_extra()`]: Submit::with_extra
    pub struct Submit<T: OpCode, E = ()> {
        // No runtime handle stored here — the runtime is obtained on demand
        // via the thread-local (`Runtime::current` / `try_with_current`).
        // Storing any form of `Rc<RuntimeInner>` (strong or weak) from inside
        // a task — which lives inside the executor — creates a reference cycle
        // that prevents `executor.clear()` from running on runtime drop,
        // leaking the io_uring fd and every fd owned by in-flight ops.
        state: Option<State<T, E>>,
    }

    impl<T: OpCode, E> PinnedDrop for Submit<T, E> {
        fn drop(this: Pin<&mut Self>) {
            let this = this.project();
            // `try_with_current` no-ops if called outside a runtime context.
            // That happens when `executor.clear()` drops tasks; it runs inside
            // `Runtime::enter`, so the thread-local IS set and the cancel goes
            // through. If somehow called after the runtime is fully gone, the
            // io_uring fd is already closed — no need to cancel.
            if let Some(State::Submitted { key, .. }) = this.state.take() {
                let _ = Runtime::try_with_current(|rt| rt.cancel(key));
            }
        }
    }

}

enum State<T: OpCode, E> {
    Idle { op: T },
    Submitted { key: Key<T>, _p: PhantomData<E> },
}

impl<T: OpCode, E> State<T, E> {
    fn submitted(key: Key<T>) -> Self {
        State::Submitted {
            key,
            _p: PhantomData,
        }
    }
}

impl<T: OpCode> Submit<T, ()> {
    pub(crate) fn new(op: T) -> Self {
        Submit {
            state: Some(State::Idle { op }),
        }
    }

    /// Convert this future to one that returns [`Extra`] along with the result.
    ///
    /// This is useful if you need to access extra information provided by the
    /// runtime upon completion of the operation.
    pub fn with_extra(mut self) -> Submit<T, Extra> {
        let Some(state) = self.state.take() else {
            return Submit { state: None };
        };
        let state = match state {
            State::Submitted { key, .. } => State::Submitted {
                key,
                _p: PhantomData,
            },
            State::Idle { op } => State::Idle { op },
        };
        Submit { state: Some(state) }
    }
}

impl<T: OpCode + 'static> Future for Submit<T, ()> {
    type Output = BufResult<usize, T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let runtime = Runtime::current();

        loop {
            match this.state.take().expect("Cannot poll after ready") {
                State::Submitted { key, .. } => match runtime.poll_task(cx.get_waker(), key) {
                    PushEntry::Pending(key) => {
                        *this.state = Some(State::submitted(key));
                        return Poll::Pending;
                    }
                    PushEntry::Ready(res) => return Poll::Ready(res),
                },
                State::Idle { op } => {
                    let extra = cx.as_extra(|| runtime.default_extra());
                    match runtime.submit_raw(op, extra) {
                        PushEntry::Pending(key) => {
                            // TODO: Should we register it only the first time or every time it's
                            // being polled?
                            if let Some(cancel) = cx.get_cancel() {
                                cancel.register(&key);
                            };

                            *this.state = Some(State::submitted(key))
                        }
                        PushEntry::Ready(res) => {
                            return Poll::Ready(res);
                        }
                    }
                }
            }
        }
    }
}

impl<T: OpCode + 'static> Future for Submit<T, Extra> {
    type Output = (BufResult<usize, T>, Extra);

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let runtime = Runtime::current();

        loop {
            match this.state.take().expect("Cannot poll after ready") {
                State::Submitted { key, .. } => {
                    match runtime.poll_task_with_extra(cx.get_waker(), key) {
                        PushEntry::Pending(key) => {
                            *this.state = Some(State::submitted(key));
                            return Poll::Pending;
                        }
                        PushEntry::Ready(res) => return Poll::Ready(res),
                    }
                }
                State::Idle { op } => {
                    let extra = cx.as_extra(|| runtime.default_extra());
                    match runtime.submit_raw(op, extra) {
                        PushEntry::Pending(key) => {
                            if let Some(cancel) = cx.get_cancel() {
                                cancel.register(&key);
                            }

                            *this.state = Some(State::submitted(key))
                        }
                        PushEntry::Ready(res) => {
                            return Poll::Ready((res, runtime.default_extra()));
                        }
                    }
                }
            }
        }
    }
}

impl<T: OpCode, E> FusedFuture for Submit<T, E>
where
    Submit<T, E>: Future,
{
    fn is_terminated(&self) -> bool {
        self.state.is_none()
    }
}
