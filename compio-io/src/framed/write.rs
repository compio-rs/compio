use std::{
    io,
    task::{Poll, ready},
};

use compio_buf::{BufResult, IoBufMut};
use futures_util::{FutureExt, Sink};

use crate::{
    AsyncWrite, AsyncWriteExt, PinBoxFuture,
    framed::{Framed, INCONSISTENT_ERROR, codec::Encoder, frame::Framer, panic_config_polled},
};

pub enum State<Io, B> {
    Configuring(Option<Io>, Option<B>),
    Idle(Option<(Io, B)>),
    Writing(PinBoxFuture<(Io, BufResult<(), B>)>),
    Closing(PinBoxFuture<(Io, io::Result<()>, B)>),
    Flushing(PinBoxFuture<(Io, io::Result<()>, B)>),
}

macro_rules! initialize {
    ($this:expr, $io:expr, $buf:expr) => {
        *$this = State::Idle(Some((
            $io.take().expect("io is empty"),
            $buf.take().expect("buf is empty"),
        )));
    };
}

impl<Io> State<Io, Vec<u8>> {
    pub fn empty() -> Self {
        State::Configuring(None, Some(Vec::new()))
    }
}

impl<Io, B> State<Io, B> {
    pub fn with_io<I>(self, io: I) -> State<I, B> {
        let State::Configuring(_, b) = self else {
            panic_config_polled()
        };
        State::Configuring(Some(io), b)
    }

    pub fn with_buf<Buf: IoBufMut>(self, buf: Buf) -> State<Io, Buf> {
        let State::Configuring(io, _) = self else {
            panic_config_polled()
        };
        State::Configuring(io, Some(buf))
    }

    fn take_idle(&mut self) -> (Io, B) {
        match self {
            State::Configuring(io, buf) => {
                initialize!(self, io, buf);
                self.take_idle()
            }
            State::Idle(idle) => idle.take().expect(INCONSISTENT_ERROR),
            _ => unreachable!("{}", INCONSISTENT_ERROR),
        }
    }

    fn buf(&mut self) -> Option<&mut B> {
        match self {
            State::Idle(Some((_, buf))) => Some(buf),
            State::Configuring(io, buf) => {
                initialize!(self, io, buf);
                self.buf()
            }
            _ => None,
        }
    }

    fn poll_sink(&mut self, cx: &mut std::task::Context<'_>) -> Poll<io::Result<()>> {
        let (io, res, buf) = match self {
            State::Configuring(io, buf) => {
                initialize!(self, io, buf);
                return Poll::Ready(Ok(()));
            }
            State::Idle(_) => {
                return Poll::Ready(Ok(()));
            }
            State::Writing(fut) => {
                let (io, BufResult(res, buf)) = ready!(fut.poll_unpin(cx));
                (io, res, buf)
            }
            State::Closing(fut) | State::Flushing(fut) => ready!(fut.poll_unpin(cx)),
        };
        *self = State::Idle(Some((io, buf)));
        Poll::Ready(res)
    }
}

impl<Io: AsyncWrite + 'static, B: IoBufMut> State<Io, B> {
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

impl<R, W, C, F, In, Out, B> Sink<In> for Framed<R, W, C, F, In, Out, B>
where
    W: AsyncWrite + 'static,
    C: Encoder<In, B>,
    F: Framer<B>,
    B: IoBufMut,
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
        let _ = buf.reserve(64);
        if let Err(e) = this.codec.encode(item, buf) {
            buf.clear();
            return Err(e);
        };
        this.framer.enclose(buf);
        this.write_state.start_write();

        Ok(())
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        let this = self.as_mut().get_mut();
        match &mut this.write_state {
            State::Configuring(io, buf) => {
                initialize!(&mut this.write_state, io, buf);
                self.poll_flush(cx)
            }
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
