use std::{
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use compio_buf::{BufResult, SetLen};
use compio_driver::{
    BufferPool, BufferRef, Extra, Key, OpCode, PushEntry, TakeBuffer,
    op::{RecvFromMultiResult, RecvMsgMultiResult},
};
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

impl<T: OpCode + TakeBuffer + 'static> SubmitMulti<T>
where
    <T as TakeBuffer>::Buffer: HandleBufferRef<Param = ()>,
{
    /// Convert this stream into one that iterates the buffers from the results.
    pub fn into_managed(self, buffer_pool: BufferPool) -> SubmitMultiManaged<T, T::Buffer> {
        SubmitMultiManaged::new(self, buffer_pool, ())
    }
}

impl<T: OpCode + TakeBuffer + 'static> SubmitMulti<T>
where
    <T as TakeBuffer>::Buffer: HandleBufferRef,
{
    /// Convert this stream into one that iterates the buffers from the results,
    /// with a param to construct the result item.
    pub fn into_managed_with(
        self,
        buffer_pool: BufferPool,
        param: <<T as TakeBuffer>::Buffer as HandleBufferRef>::Param,
    ) -> SubmitMultiManaged<T, T::Buffer> {
        SubmitMultiManaged::new(self, buffer_pool, param)
    }
}

/// A wrapper around [`SubmitMulti`] that iterates the buffers from the results.
pub struct SubmitMultiManaged<T: OpCode, B = BufferRef>
where
    B: HandleBufferRef + 'static,
{
    inner: Option<SubmitMulti<T>>,
    buffer_pool: BufferPool,
    param: <B as HandleBufferRef>::Param,
    _p: PhantomData<&'static B>,
}

impl<T: OpCode, B: HandleBufferRef + 'static> SubmitMultiManaged<T, B> {
    fn new(
        stream: SubmitMulti<T>,
        buffer_pool: BufferPool,
        param: <B as HandleBufferRef>::Param,
    ) -> Self {
        Self {
            inner: Some(stream),
            buffer_pool,
            param,
            _p: PhantomData,
        }
    }
}

impl<T: OpCode + TakeBuffer<Buffer = B> + 'static, B: HandleBufferRef> Stream
    for SubmitMultiManaged<T, B>
{
    type Item = std::io::Result<B>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Some(inner) = self.inner.as_mut() {
            let buffer = match std::task::ready!(inner.poll_next_unpin(cx)) {
                Some(BufResult(res, extra)) => {
                    if inner.is_terminated() {
                        let mut b = self
                            .inner
                            .take()
                            .and_then(|s| s.try_take().ok())
                            .and_then(|op| op.take_buffer());
                        if let Some(ref mut b) = b {
                            unsafe { b.advance_to(res?) }
                        }
                        b
                    } else {
                        let b = self.buffer_pool.take(extra.buffer_id()?)?;
                        if let Some(mut b) = b {
                            unsafe {
                                SetLen::advance_to(&mut b, res?);
                                Some(B::from_buffer_ref(b, self.param))
                            }
                        } else {
                            None
                        }
                    }
                }
                None => self
                    .inner
                    .take()
                    .and_then(|s| s.try_take().ok())
                    .and_then(|op| op.take_buffer()),
            };
            Poll::Ready(buffer.map(Ok))
        } else {
            Poll::Ready(None)
        }
    }
}

impl<T: OpCode + TakeBuffer<Buffer = B> + 'static, B: HandleBufferRef> FusedStream
    for SubmitMultiManaged<T, B>
{
    fn is_terminated(&self) -> bool {
        self.inner.as_ref().is_none_or(|s| s.is_terminated())
    }
}

mod private {
    use super::*;

    pub trait Sealed {}

    impl Sealed for BufferRef {}
    impl Sealed for RecvFromMultiResult {}
    impl Sealed for RecvMsgMultiResult {}
}

#[doc(hidden)]
pub trait HandleBufferRef: private::Sealed {
    type Param: Copy + Unpin;

    unsafe fn from_buffer_ref(buffer: BufferRef, param: Self::Param) -> Self;

    unsafe fn advance_to(&mut self, len: usize);
}

impl HandleBufferRef for BufferRef {
    type Param = ();

    unsafe fn from_buffer_ref(buffer: BufferRef, _: Self::Param) -> Self {
        buffer
    }

    unsafe fn advance_to(&mut self, len: usize) {
        unsafe { SetLen::advance_to(self, len) }
    }
}

impl HandleBufferRef for RecvFromMultiResult {
    type Param = ();

    unsafe fn from_buffer_ref(buffer: BufferRef, _: Self::Param) -> Self {
        unsafe { RecvFromMultiResult::new(buffer) }
    }

    unsafe fn advance_to(&mut self, _: usize) {}
}

impl HandleBufferRef for RecvMsgMultiResult {
    type Param = usize;

    unsafe fn from_buffer_ref(buffer: BufferRef, clen: usize) -> Self {
        unsafe { RecvMsgMultiResult::new(buffer, clen) }
    }

    unsafe fn advance_to(&mut self, _: usize) {}
}
