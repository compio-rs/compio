use std::{
    ops::Deref,
    pin::Pin,
    task::{Context, Poll},
};

use compio_io::{AsyncRead, AsyncWrite};
use futures_util::{Sink, Stream};
use tungstenite::Message;

use crate::{WebSocketStream, WsError};

type PinBoxFuture<T> = Pin<Box<dyn Future<Output = T>>>;

/// A [`futures_util`] compatible WebSocket stream.
pub struct CompatWebSocketStream<S> {
    inner: Pin<Box<WebSocketStream<S>>>,
    read_future: Option<PinBoxFuture<Result<Message, WsError>>>,
    write_future: Option<PinBoxFuture<Result<(), WsError>>>,
    close_future: Option<PinBoxFuture<Result<(), WsError>>>,
}

impl<S> CompatWebSocketStream<S> {
    pub(super) fn new(stream: WebSocketStream<S>) -> Self {
        Self {
            inner: Box::pin(stream),
            read_future: None,
            write_future: None,
            close_future: None,
        }
    }
}

impl<S> Deref for CompatWebSocketStream<S> {
    type Target = WebSocketStream<S>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

macro_rules! poll_future {
    ($f:expr, $cx:expr, $e:expr) => {{
        let mut future = match $f.take() {
            Some(f) => f,
            None => Box::pin($e),
        };
        let f = future.as_mut();
        match f.poll($cx) {
            Poll::Pending => {
                $f.replace(future);
                return Poll::Pending;
            }
            Poll::Ready(res) => res,
        }
    }};
}

impl<S: AsyncRead + AsyncWrite + 'static> Sink<Message> for CompatWebSocketStream<S> {
    type Error = tungstenite::Error;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if let Some(mut fut) = self.write_future.take() {
            match fut.as_mut().poll(cx) {
                Poll::Pending => {
                    self.write_future.replace(fut);
                    Poll::Pending
                }
                Poll::Ready(Ok(_)) => Poll::Ready(Ok(())),
                Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            }
        } else if let Some(mut fut) = self.close_future.take() {
            match fut.as_mut().poll(cx) {
                Poll::Pending => {
                    self.close_future.replace(fut);
                    Poll::Pending
                }
                Poll::Ready(Ok(_)) => Poll::Ready(Ok(())),
                Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            }
        } else {
            Poll::Ready(Ok(()))
        }
    }

    fn start_send(mut self: Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
        let inner: &'static mut WebSocketStream<S> =
            unsafe { &mut *(self.inner.as_mut().get_unchecked_mut() as *mut _) };
        match inner.inner.write(item) {
            Ok(()) => Ok(()),
            Err(WsError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                self.write_future.replace(Box::pin(inner.flush()));
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.close_future.is_some() {
            debug_assert!(self.write_future.is_none());
            return Poll::Pending;
        }

        let inner: &'static mut WebSocketStream<S> =
            unsafe { &mut *(self.inner.as_mut().get_unchecked_mut() as *mut _) };
        let res = poll_future!(self.write_future, cx, inner.flush());
        Poll::Ready(res)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.write_future.is_some() || self.inner.inner.get_ref().has_pending_write() {
            debug_assert!(self.close_future.is_none());
            self.poll_flush(cx)
        } else {
            let inner: &'static mut WebSocketStream<S> =
                unsafe { &mut *(self.inner.as_mut().get_unchecked_mut() as *mut _) };
            let res = poll_future!(self.close_future, cx, inner.close(None));
            Poll::Ready(res)
        }
    }
}

impl<S: AsyncRead + AsyncWrite + 'static> Stream for CompatWebSocketStream<S> {
    type Item = Result<Message, WsError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let inner: &'static mut WebSocketStream<S> =
            unsafe { &mut *(self.inner.as_mut().get_unchecked_mut() as *mut _) };

        let res = poll_future!(self.read_future, cx, inner.read());
        match res {
            Ok(msg) => Poll::Ready(Some(Ok(msg))),
            Err(WsError::AlreadyClosed) | Err(WsError::ConnectionClosed) => Poll::Ready(None),
            Err(e) => Poll::Ready(Some(Err(e))),
        }
    }
}
