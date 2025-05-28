//! Framed I/O operations.
//!
//! This module provides functionality for encoding and decoding frames
//! for network protocols and other stream-based communication.

use std::{io, marker::PhantomData, task::Poll};

use compio_buf::{BufResult, IntoInner, IoBuf, Uninit};
use futures_util::{FutureExt, Sink, Stream, ready};

use crate::{
    AsyncRead, AsyncWrite, AsyncWriteExt, PinBoxFuture,
    framed::{
        codec::{Decoder, Encoder},
        frame::Framer,
    },
};

pub mod codec;
pub mod frame;

type ReadResult = BufResult<usize, Uninit<Vec<u8>>>;

enum State<Io> {
    Idle(Option<(Io, Vec<u8>)>),
    Writing(PinBoxFuture<(Io, BufResult<(), Vec<u8>>)>),
    Closing(PinBoxFuture<(Io, io::Result<()>, Vec<u8>)>),
    Flushing(PinBoxFuture<(Io, io::Result<()>, Vec<u8>)>),
    Reading(PinBoxFuture<(Io, ReadResult)>),
}

impl<Io> State<Io> {
    fn take_idle(&mut self) -> (Io, Vec<u8>) {
        match self {
            State::Idle(idle) => idle.take().expect("Inconsistent state"),
            _ => unreachable!("`Framed` not in idle state"),
        }
    }

    #[cfg(test)]
    fn io(&mut self) -> Option<&mut Io> {
        match self {
            State::Idle(Some((io, _))) => Some(io),
            _ => None,
        }
    }

    fn buf(&mut self) -> Option<&mut Vec<u8>> {
        match self {
            State::Idle(Some((_, buf))) => Some(buf),
            _ => None,
        }
    }

    fn start_flush(&mut self)
    where
        Io: AsyncWrite + 'static,
    {
        let (mut io, buf) = self.take_idle();
        let fut = Box::pin(async move {
            let res = io.flush().await;
            (io, res, buf)
        });
        *self = State::Flushing(fut);
    }

    fn start_close(&mut self)
    where
        Io: AsyncWrite + 'static,
    {
        let (mut io, buf) = self.take_idle();
        let fut = Box::pin(async move {
            let res = io.shutdown().await;
            (io, res, buf)
        });
        *self = State::Closing(fut);
    }

    fn start_write(&mut self)
    where
        Io: AsyncWrite + 'static,
    {
        let (mut io, buf) = self.take_idle();
        let fut = Box::pin(async move {
            let res = io.write_all(buf).await;
            (io, res)
        });
        *self = State::Writing(fut);
    }

    /// State that may occur when `Framed` is acting as a [`Sink`].
    fn poll_sink(&mut self, cx: &mut std::task::Context<'_>) -> Poll<io::Result<()>> {
        let (io, res, buf) = match self {
            State::Writing(fut) => {
                let (io, BufResult(res, buf)) = ready!(fut.poll_unpin(cx));
                (io, res, buf)
            }

            State::Closing(fut) | State::Flushing(fut) => ready!(fut.poll_unpin(cx)),
            State::Idle(_) | State::Reading(_) => unreachable!("`Framed` cannot be polled"),
        };
        *self = State::Idle(Some((io, buf)));
        Poll::Ready(res)
    }
}

/// A framed encoder/decoder that handles both [`Sink`] for writing frames and
/// [`Stream`] for reading frames.
///
/// It uses a [`codec`] to encode/decode messages and a [`Framer`] to
/// define how frames are laid out in buffer.
pub struct Framed<Io, C, F, In, Out> {
    state: State<Io>,
    codec: C,
    framer: F,
    types: PhantomData<(In, Out)>,
}

impl<Io, C, F, In, Out> Framed<Io, C, F, In, Out> {
    /// Creates a new `Framed` with the given I/O object, codec, and framer.
    pub fn new(io: Io, codec: C, framer: F) -> Self {
        Self {
            state: State::Idle(Some((io, Vec::new()))),
            codec,
            framer,
            types: PhantomData,
        }
    }
}

impl<Io, C, F> Framed<Io, C, F, (), ()> {
    /// Creates a new `Framed` with the given I/O object, codec, and framer with
    /// symmetric In/Out type.
    pub fn symmetric<T>(io: Io, codec: C, framer: F) -> Framed<Io, C, F, T, T> {
        Framed {
            state: State::Idle(Some((io, Vec::new()))),
            codec,
            framer,
            types: PhantomData,
        }
    }
}

impl<Io, C, F, In, Out> Sink<In> for Framed<Io, C, F, In, Out>
where
    Io: AsyncWrite + 'static,
    C: Encoder<In>,
    F: Framer,
    Self: Unpin,
{
    type Error = C::Error;

    fn poll_ready(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        let this = self.get_mut();
        match &mut this.state {
            State::Idle(..) => Poll::Ready(Ok(())),
            State::Reading(fut) => {
                // If we are reading, we need to finish the read operation first
                let (io, BufResult(res, buf)) = ready!(fut.poll_unpin(cx));
                this.state = State::Idle(Some((io, buf.into_inner())));
                Poll::Ready(res.map_err(C::Error::from).map(|_| ()))
            }
            state => state.poll_sink(cx).map_err(C::Error::from),
        }
    }

    fn start_send(self: std::pin::Pin<&mut Self>, item: In) -> Result<(), Self::Error> {
        let this = self.get_mut();

        let buf = this.state.buf().expect("`Framed` not in idle state");
        buf.clear();
        buf.reserve(64);
        this.codec.encode(item, buf)?;
        this.framer.enclose(buf);
        this.state.start_write();

        Ok(())
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        let this = self.get_mut();
        match &mut this.state {
            State::Idle(_) => {
                this.state.start_flush();
                this.state.poll_sink(cx).map_err(C::Error::from)
            }
            State::Writing(_) | State::Flushing(_) => {
                this.state.poll_sink(cx).map_err(C::Error::from)
            }
            _ => unreachable!("`Framed` not able to flush"),
        }
    }

    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        let this = self.get_mut();
        match &mut this.state {
            state @ State::Idle(_) => {
                state.start_close();
                state.poll_sink(cx).map_err(C::Error::from)
            }
            state @ State::Closing(_) => state.poll_sink(cx).map_err(C::Error::from),
            _ => unreachable!("`Framed` not able to flush"),
        }
    }
}

impl<Io, C, F, In, Out> Stream for Framed<Io, C, F, In, Out>
where
    Io: AsyncRead + 'static,
    C: Decoder<Out>,
    F: frame::Framer,
    Self: Unpin,
{
    type Item = Result<Out, C::Error>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        loop {
            match &mut this.state {
                State::Idle(idle) => {
                    let (mut io, mut buf) = idle.take().expect("Inconsistent state");
                    // First try decode from the buffer
                    if let Some(frame) = this.framer.extract(&buf) {
                        let decoded = this.codec.decode(frame.payload(&buf))?;
                        frame.consume(&mut buf);

                        return Poll::Ready(Some(Ok(decoded)));
                    }

                    // If nothing can be decoded, read more data
                    buf.reserve(64);
                    let fut = Box::pin(async move {
                        let res = io.read(buf.uninit()).await; // Only write data to uninitialized area
                        (io, res)
                    });
                    this.state = State::Reading(fut)
                }
                State::Reading(fut) => {
                    let (io, BufResult(res, buf)) = ready!(fut.poll_unpin(cx));
                    this.state = State::Idle(Some((io, buf.into_inner())));
                    res?;
                }
                _ => unreachable!("`Framed` not in reading state"),
            };
        }
    }
}

#[cfg(test)]
mod test {
    use std::io::Cursor;

    use futures_util::{SinkExt, StreamExt};
    use serde::{Deserialize, Serialize};

    use crate::framed::{Framed, codec::serde_json::SerdeJsonCodec, frame::LengthDelimited};

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    struct Test {
        foo: String,
        bar: usize,
    }

    #[compio_macros::test]
    async fn test_framed() {
        let codec = SerdeJsonCodec::new();
        let framer = LengthDelimited::new();
        let mut framed = Framed::symmetric::<Test>(Cursor::new(Vec::new()), codec, framer);

        let origin = Test {
            foo: "hello, world!".to_owned(),
            bar: 114514,
        };
        framed.send(origin.clone()).await.unwrap();

        let io = framed.state.io().expect("Invalid state");
        println!(
            "{:?}",
            std::str::from_utf8(io.get_ref().as_slice()).unwrap()
        );

        let des = framed.next().await.unwrap().unwrap();
        println!("{des:?}");

        assert_eq!(origin, des);
    }
}
