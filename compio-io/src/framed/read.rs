use std::{
    pin::Pin,
    task::{Context, Poll, ready},
};

use compio_buf::{BufResult, IntoInner, IoBufMut};
use futures_util::Stream;

use super::*;
use crate::{AsyncReadExt, PinBoxFuture, buffer::Buffer, framed::frame::Framer};

type ReadResult<B> = BufResult<usize, Buffer<B>>;

pub struct State<Io, B> {
    inner: StateInner<Io, B>,
    eof: bool,
}

impl<Io> State<Io, Vec<u8>> {
    pub fn empty() -> Self {
        State {
            inner: StateInner::Configuring(None, Some(Buffer::new())),
            eof: false,
        }
    }
}

impl<Io, B> State<Io, B> {
    pub fn with_io<I>(self, io: I) -> State<I, B> {
        let StateInner::Configuring(_, b) = self.inner else {
            panic_config_polled()
        };
        State {
            inner: StateInner::Configuring(Some(io), b),
            eof: false,
        }
    }

    pub fn with_buf<Buf: IoBufMut>(self, buf: Buf) -> State<Io, Buf> {
        let StateInner::Configuring(io, _) = self.inner else {
            panic_config_polled()
        };
        State {
            inner: StateInner::Configuring(io, Some(Buffer::new_with(buf))),
            eof: false,
        }
    }
}

enum StateInner<Io, B> {
    Configuring(Option<Io>, Option<Buffer<B>>),
    Idle(Option<(Io, Buffer<B>)>),
    Reading(PinBoxFuture<(Io, ReadResult<B>)>),
}

impl<R, W, C, F, In, Out, B> Stream for Framed<R, W, C, F, In, Out, B>
where
    R: AsyncRead + 'static,
    C: Decoder<Out, B>,
    F: Framer<B>,
    B: IoBufMut,
    Self: Unpin,
{
    type Item = Result<Out, C::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        loop {
            match &mut this.read_state.inner {
                StateInner::Configuring(io, buf) => {
                    let io = io.take().expect("Inconsistent state");
                    let buf = buf.take().expect("Inconsistent state");
                    this.read_state.inner = StateInner::Idle(Some((io, buf)));
                }
                StateInner::Idle(idle) => {
                    let (mut io, mut buf) = idle.take().expect("Inconsistent state");

                    // First try decode from the buffer
                    let inner = buf.inner();
                    if let Some(frame) = this.framer.extract(inner)? {
                        let (begin, end) = (inner.begin(), inner.end());
                        let slice = frame.slice(buf.take_inner()).flatten(); // focus on only the payload
                        let decoded = this.codec.decode(&slice);
                        let inner = slice.into_inner();
                        if let Some(end) = end {
                            buf.restore_inner(inner.slice(begin..end));
                        } else {
                            buf.restore_inner(inner.slice(begin..));
                        }

                        if buf.advance(frame.len()) {
                            buf.reset();
                        }

                        this.read_state.inner = StateInner::Idle(Some((io, buf)));

                        return Poll::Ready(Some(decoded));
                    }

                    buf.reserve(16);

                    let fut = Box::pin(async move {
                        let res = buf.with(|buf| io.append(buf)).await;
                        (io, BufResult(res, buf))
                    });

                    this.read_state.inner = StateInner::Reading(fut)
                }
                StateInner::Reading(fut) => {
                    let (io, BufResult(res, buf)) = ready!(fut.poll_unpin(cx));
                    this.read_state.inner = StateInner::Idle(Some((io, buf)));
                    if res? == 0 {
                        // It's the second time EOF is reached, return None
                        if this.read_state.eof {
                            return Poll::Ready(None);
                        }

                        this.read_state.eof = true;
                    }
                }
            };
        }
    }
}
