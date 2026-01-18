//! Future for submitting operations to the runtime.

use std::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use compio_buf::BufResult;
use compio_driver::{Extra, Key, OpCode, PushEntry};
use futures_util::future::FusedFuture;

use crate::runtime::Runtime;

trait ContextExt {
    fn as_extra(&mut self, extra: impl FnOnce() -> Extra) -> Option<Extra>;
}

impl ContextExt for Context<'_> {
    fn as_extra(&mut self, extra: impl FnOnce() -> Extra) -> Option<Extra> {
        let _ = extra;
        None
    }
}

/// Return type for `Runtime::submit`
///
/// By default, this implements `Future<Output = BufResult<usize, T>>`. If
/// [`Extra`] is needed, call [`.with_extra()`] to get a `Submit<T, Extra>`
/// which implements `Future<Output = (BufResult<usize, T>, Extra)>`.
///
/// [`.with_extra()`]: Submit::with_extra
pub struct Submit<T: OpCode, E = ()> {
    runtime: Runtime,
    state: Option<State<T, E>>,
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
    pub(crate) fn new(runtime: Runtime, op: T) -> Self {
        Submit {
            runtime,
            state: Some(State::Idle { op }),
        }
    }

    /// Convert this future to one that returns [`Extra`] along with the result.
    ///
    /// This is useful if you need to access extra information provided by the
    /// runtime upon completion of the operation.
    pub fn with_extra(mut self) -> Submit<T, Extra> {
        let runtime = self.runtime.clone();
        let Some(state) = self.state.take() else {
            return Submit {
                runtime,
                state: None,
            };
        };
        let state = match state {
            State::Submitted { key, .. } => State::Submitted {
                key,
                _p: PhantomData,
            },
            State::Idle { op } => State::Idle { op },
        };
        Submit {
            runtime,
            state: Some(state),
        }
    }
}

impl<T: OpCode + 'static> Future for Submit<T, ()> {
    type Output = BufResult<usize, T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };
        loop {
            match this.state.take().expect("Cannot poll after ready") {
                State::Submitted { key, .. } => match this.runtime.poll_task(cx.waker(), key) {
                    PushEntry::Pending(key) => {
                        this.state = Some(State::submitted(key));
                        return Poll::Pending;
                    }
                    PushEntry::Ready(res) => return Poll::Ready(res),
                },
                State::Idle { op } => {
                    let extra = cx.as_extra(|| this.runtime.default_extra());
                    match this.runtime.submit_raw(op, extra) {
                        PushEntry::Pending(key) => this.state = Some(State::submitted(key)),
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
        let this = unsafe { self.get_unchecked_mut() };
        loop {
            match this.state.take().expect("Cannot poll after ready") {
                State::Submitted { key, .. } => match this.runtime.poll_task_with_extra(cx, key) {
                    PushEntry::Pending(key) => {
                        this.state = Some(State::submitted(key));
                        return Poll::Pending;
                    }
                    PushEntry::Ready(res) => return Poll::Ready(res),
                },
                State::Idle { op } => {
                    let extra = cx.as_extra(|| this.runtime.default_extra());
                    match this.runtime.submit_raw(op, extra) {
                        PushEntry::Pending(key) => this.state = Some(State::submitted(key)),
                        PushEntry::Ready(res) => {
                            return Poll::Ready((res, this.runtime.default_extra()));
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

impl<T: OpCode, E> Drop for Submit<T, E> {
    fn drop(&mut self) {
        if let Some(State::Submitted { key, .. }) = self.state.take() {
            self.runtime.cancel(key);
        }
    }
}
