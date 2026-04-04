use std::{
    pin::Pin,
    task::{Context, Poll},
};

use compio_buf::{BufResult, SetLen};
use compio_driver::{BufferPool, BufferRef, Extra, Key, OpCode, PushEntry, TakeBuffer};
use futures_util::{Stream, StreamExt, stream::FusedStream};

use crate::{ContextExt, Runtime};

pin_project_lite::pin_project! {
    /// Returned [`Stream`] for [`Runtime::submit_multi`].
    ///
    /// When this is dropped and the operation hasn't finished yet, it will try to
    /// cancel the operation.
    pub struct SubmitMulti<T: OpCode> {
        runtime: Runtime,
        state: Option<State<T>>,
    }

    impl<T: OpCode> PinnedDrop for SubmitMulti<T> {
        fn drop(this: Pin<&mut Self>) {
            let this = this.project();
            if let Some(State::Submitted { key }) = this.state.take() {
                this.runtime.cancel(key);
            }
        }
    }
}

enum State<T: OpCode> {
    Idle { op: T },
    Submitted { key: Key<T> },
    Finished { op: T },
}

impl<T: OpCode> State<T> {
    fn submitted(key: Key<T>) -> Self {
        State::Submitted { key }
    }
}

impl<T: OpCode> SubmitMulti<T> {
    pub(crate) fn new(runtime: Runtime, op: T) -> Self {
        SubmitMulti {
            runtime,
            state: Some(State::Idle { op }),
        }
    }

    /// Try to take the inner op from the stream.
    ///
    /// Returns `Ok(T)` if the stream:
    ///
    /// - has not been polled yet, or
    /// - is finished and the op is returned by the driver
    ///
    /// Returns `Err(Self)` if it's still running.
    pub fn try_take(mut self) -> Result<T, Self> {
        match self.state.take() {
            Some(State::Finished { op }) | Some(State::Idle { op }) => Ok(op),
            state => {
                debug_assert!(state.is_some());
                self.state = state;
                Err(self)
            }
        }
    }
}

impl<T: OpCode + 'static> Stream for SubmitMulti<T> {
    type Item = BufResult<usize, Extra>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();

        loop {
            match this.state.take().expect("State error, this is a bug") {
                State::Idle { op } => {
                    let extra = cx.as_extra(|| this.runtime.default_extra());
                    match this.runtime.submit_raw(op, extra) {
                        PushEntry::Pending(key) => {
                            if let Some(cancel) = cx.get_cancel() {
                                cancel.register(&key);
                            }

                            *this.state = Some(State::submitted(key))
                        }
                        PushEntry::Ready(BufResult(res, op)) => {
                            *this.state = Some(State::Finished { op });
                            let extra = this.runtime.default_extra();

                            return Poll::Ready(Some(BufResult(res, extra)));
                        }
                    }
                }

                State::Submitted { key, .. } => {
                    if let Some(res) = this.runtime.poll_multishot(cx.get_waker(), &key) {
                        *this.state = Some(State::submitted(key));

                        return Poll::Ready(Some(res));
                    };

                    match this.runtime.poll_task_with_extra(cx.get_waker(), key) {
                        PushEntry::Pending(key) => {
                            *this.state = Some(State::submitted(key));

                            return Poll::Pending;
                        }
                        PushEntry::Ready((BufResult(res, op), extra)) => {
                            *this.state = Some(State::Finished { op });

                            return Poll::Ready(Some(BufResult(res, extra)));
                        }
                    }
                }

                State::Finished { op } => {
                    *this.state = Some(State::Finished { op });

                    return Poll::Ready(None);
                }
            }
        }
    }
}

impl<T: OpCode + 'static> FusedStream for SubmitMulti<T> {
    fn is_terminated(&self) -> bool {
        matches!(self.state, None | Some(State::Finished { .. }))
    }
}

impl<T: OpCode + TakeBuffer<Buffer = BufferRef> + 'static> SubmitMulti<T> {
    /// Convert this stream into one that iterates the buffers from the results.
    pub fn into_managed(self, buffer_pool: BufferPool) -> SubmitMultiManaged<T> {
        SubmitMultiManaged::new(self, buffer_pool)
    }
}

/// A wrapper around [`SubmitMulti`] that iterates the buffers from the results.
pub struct SubmitMultiManaged<T: OpCode> {
    inner: Option<SubmitMulti<T>>,
    buffer_pool: BufferPool,
}

impl<T: OpCode> SubmitMultiManaged<T> {
    fn new(stream: SubmitMulti<T>, buffer_pool: BufferPool) -> Self {
        Self {
            inner: Some(stream),
            buffer_pool,
        }
    }
}

impl<T: OpCode + TakeBuffer<Buffer = BufferRef> + 'static> Stream for SubmitMultiManaged<T> {
    type Item = std::io::Result<BufferRef>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Some(inner) = self.inner.as_mut() {
            let (mut buffer, res) = match std::task::ready!(inner.poll_next_unpin(cx)) {
                Some(BufResult(res, extra)) => {
                    let buffer = if inner.is_terminated() {
                        self.inner
                            .take()
                            .and_then(|s| s.try_take().ok())
                            .and_then(|op| op.take_buffer())
                    } else {
                        self.buffer_pool.take(extra.buffer_id()?)?
                    };
                    (buffer, res)
                }
                None => (
                    self.inner
                        .take()
                        .and_then(|s| s.try_take().ok())
                        .and_then(|op| op.take_buffer()),
                    Ok(0),
                ),
            };
            if let Some(buf) = &mut buffer {
                unsafe { buf.advance_to(res?) }
            }
            Poll::Ready(buffer.map(Ok))
        } else {
            Poll::Ready(None)
        }
    }
}

impl<T: OpCode + TakeBuffer<Buffer = BufferRef> + 'static> FusedStream for SubmitMultiManaged<T> {
    fn is_terminated(&self) -> bool {
        self.inner.as_ref().is_none_or(|s| s.is_terminated())
    }
}
