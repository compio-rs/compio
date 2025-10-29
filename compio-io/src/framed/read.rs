use std::{
    pin::Pin,
    task::{Context, Poll, ready},
};

use compio_buf::BufResult;
use futures_util::Stream;

use super::*;
use crate::{AsyncReadExt, PinBoxFuture, buffer::Buffer, framed::frame::Framer};

type ReadResult = BufResult<usize, Buffer>;

pub struct State<Io> {
    inner: StateInner<Io>,
    eof: bool,
}

impl<Io> State<Io> {
    pub fn new(io: Io, buf: Buffer) -> Self {
        State {
            inner: StateInner::Idle(Some((io, buf))),
            eof: false,
        }
    }

    pub fn empty() -> Self {
        State {
            inner: StateInner::Idle(None),
            eof: false,
        }
    }
}

enum StateInner<Io> {
    Idle(Option<(Io, Buffer)>),
    Reading(PinBoxFuture<(Io, ReadResult)>),
}

impl<R, W, C, F, In, Out> Stream for Framed<R, W, C, F, In, Out>
where
    R: AsyncRead + 'static,
    C: Decoder<Out>,
    F: Framer,
    Self: Unpin,
{
    type Item = Result<Out, C::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        loop {
            match &mut this.read_state.inner {
                StateInner::Idle(idle) => {
                    let (mut io, mut buf) = idle.take().expect("Inconsistent state");
                    let slice = buf.slice();

                    // First try decode from the buffer
                    if let Some(frame) = this.framer.extract(slice) {
                        let decoded = this.codec.decode(frame.payload(slice))?;
                        buf.advance(frame.len());

                        if buf.all_done() {
                            buf.reset();
                        }

                        this.read_state.inner = StateInner::Idle(Some((io, buf)));

                        return Poll::Ready(Some(Ok(decoded)));
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
