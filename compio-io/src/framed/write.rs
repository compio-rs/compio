use std::{
    io,
    task::{Poll, ready},
};

use compio_buf::BufResult;
use futures_util::{FutureExt, Sink};

use crate::{
    AsyncWrite, AsyncWriteExt, PinBoxFuture,
    framed::{Framed, codec::Encoder, frame::Framer},
};

pub enum State<Io> {
    Idle(Option<(Io, Vec<u8>)>),
    Writing(PinBoxFuture<(Io, BufResult<(), Vec<u8>>)>),
    Closing(PinBoxFuture<(Io, io::Result<()>, Vec<u8>)>),
    Flushing(PinBoxFuture<(Io, io::Result<()>, Vec<u8>)>),
}

impl<Io> State<Io> {
    pub fn new(io: Io, buf: Vec<u8>) -> Self {
        State::Idle(Some((io, buf)))
    }

    pub fn empty() -> Self {
        State::Idle(None)
    }

    fn take_idle(&mut self) -> (Io, Vec<u8>) {
        match self {
            State::Idle(idle) => idle.take().expect("Inconsistent state"),
            _ => unreachable!("`Framed` not in idle state"),
        }
    }

    fn buf(&mut self) -> Option<&mut Vec<u8>> {
        match self {
            State::Idle(Some((_, buf))) => Some(buf),
            _ => None,
        }
    }

    fn poll_sink(&mut self, cx: &mut std::task::Context<'_>) -> Poll<io::Result<()>> {
        let (io, res, buf) = match self {
            State::Writing(fut) => {
                let (io, BufResult(res, buf)) = ready!(fut.poll_unpin(cx));
                (io, res, buf)
            }
            State::Closing(fut) | State::Flushing(fut) => ready!(fut.poll_unpin(cx)),
            State::Idle(_) => {
                return Poll::Ready(Ok(()));
            }
        };
        *self = State::Idle(Some((io, buf)));
        Poll::Ready(res)
    }
}

impl<Io: AsyncWrite + 'static> State<Io> {
    fn start_flush(&mut self) {
        let (mut io, buf) = self.take_idle();
        let fut = Box::pin(async move {
            let res = io.flush().await;
            (io, res, buf)
        });
        *self = State::Flushing(fut);
    }

    fn start_close(&mut self) {
        let (mut io, buf) = self.take_idle();
        let fut = Box::pin(async move {
            let res = io.shutdown().await;
            (io, res, buf)
        });
        *self = State::Closing(fut);
    }

    fn start_write(&mut self) {
        let (mut io, buf) = self.take_idle();
        let fut = Box::pin(async move {
            let res = io.write_all(buf).await;
            (io, res)
        });
        *self = State::Writing(fut);
    }
}

impl<R, W, C, F, In, Out> Sink<In> for Framed<R, W, C, F, In, Out>
where
    W: AsyncWrite + 'static,
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
        match &mut this.write_state {
            State::Idle(..) => Poll::Ready(Ok(())),
            state => state.poll_sink(cx).map_err(C::Error::from),
        }
    }

    fn start_send(self: std::pin::Pin<&mut Self>, item: In) -> Result<(), Self::Error> {
        let this = self.get_mut();

        let buf = this.write_state.buf().expect("`Framed` not in idle state");
        buf.clear();
        buf.reserve(64);
        this.codec.encode(item, buf)?;
        this.framer.enclose(buf);
        this.write_state.start_write();

        Ok(())
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        let this = self.get_mut();
        match &mut this.write_state {
            State::Idle(_) => {
                this.write_state.start_flush();
                this.write_state.poll_sink(cx).map_err(C::Error::from)
            }
            State::Writing(_) | State::Flushing(_) => {
                this.write_state.poll_sink(cx).map_err(C::Error::from)
            }
            State::Closing(_) => unreachable!("`Framed` is closing, cannot flush"),
        }
    }

    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        let this = self.get_mut();
        match &mut this.write_state {
            state @ State::Idle(_) => {
                state.start_close();
                state.poll_sink(cx).map_err(C::Error::from)
            }
            _ => this.write_state.poll_sink(cx).map_err(C::Error::from),
        }
    }
}
