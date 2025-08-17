use std::{
    pin::Pin,
    task::{Context, Poll, ready},
};

use compio_buf::BufResult;
use futures_util::Stream;

use super::*;
use crate::{AsyncReadExt, PinBoxFuture, buffer::Buffer};

type ReadResult = BufResult<usize, Buffer>;

pub enum State<Io> {
    Idle(Option<(Io, Buffer)>),
    Reading(PinBoxFuture<(Io, ReadResult)>),
}

impl<R, W, C, F, In, Out> Stream for Framed<R, W, C, F, In, Out>
where
    R: AsyncRead + 'static,
    C: Decoder<Out>,
    F: frame::Framer,
    Self: Unpin,
{
    type Item = Result<Out, C::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        loop {
            match &mut this.read_state {
                State::Idle(idle) => {
                    let (mut io, mut buf) = idle.take().expect("Inconsistent state");

                    // First try decode from the buffer
                    if let Some(frame) = this.framer.extract(buf.slice()) {
                        let decoded = this.codec.decode(frame.payload(buf.slice()))?;
                        buf.advance(frame.len());

                        if buf.all_done() {
                            buf.reset();
                        }

                        this.read_state = State::Idle(Some((io, buf)));

                        return Poll::Ready(Some(Ok(decoded)));
                    }

                    buf.reserve(16);

                    let fut = Box::pin(async move {
                        let res = buf.with(|buf| io.append(buf)).await;
                        (io, BufResult(res, buf))
                    });

                    this.read_state = State::Reading(fut)
                }
                State::Reading(fut) => {
                    let (io, BufResult(res, buf)) = ready!(fut.poll_unpin(cx));
                    this.read_state = State::Idle(Some((io, buf)));
                    res?;
                }
            };
        }
    }
}
