use std::{
    ops::Deref,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
};

use compio_io::{AsyncRead, AsyncWrite};
use futures_util::{Sink, Stream};
use tungstenite::Message;

use crate::{WebSocketStream, WsError};

type PinBoxFuture<T> = Pin<Box<dyn Future<Output = T>>>;

/// A [`futures_util`] compatible WebSocket stream.
pub struct CompatWebSocketStream<S> {
    inner: Pin<Box<WebSocketStream<S>>>,
    read_future: Option<PinBoxFuture<Result<usize, WsError>>>,
    write_future: Option<PinBoxFuture<Result<usize, WsError>>>,
    flush_waker: Option<Waker>,
    flushed: bool,
}

impl<S> CompatWebSocketStream<S> {
    pub(super) fn new(stream: WebSocketStream<S>) -> Self {
        Self {
            inner: Box::pin(stream),
            read_future: None,
            write_future: None,
            flush_waker: None,
            flushed: false,
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

macro_rules! poll_future_would_block {
    ($f:expr, $cx:expr, $e:expr, $io:expr) => {{
        if let Some(mut f) = $f.take() {
            if f.as_mut().poll($cx).is_pending() {
                $f.replace(f);
                return Poll::Pending;
            }
        }

        match $io {
            Ok(len) => Poll::Ready(Ok(len)),
            Err(WsError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                $f.replace(Box::pin($e));
                $cx.waker().wake_by_ref();
                Poll::Pending
            }
            Err(e) => Poll::Ready(Err(e)),
        }
    }};
}

impl<S: AsyncRead + AsyncWrite + 'static> CompatWebSocketStream<S> {
    fn poll_flush_impl(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<usize, WsError>> {
        let inner: &'static mut WebSocketStream<S> =
            unsafe { &mut *(self.inner.as_mut().get_unchecked_mut() as *mut _) };
        let res = poll_future!(self.write_future, cx, inner.flush_write_buf());
        Poll::Ready(res)
    }

    fn poll_fill_impl(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<usize, WsError>> {
        let inner: &'static mut WebSocketStream<S> =
            unsafe { &mut *(self.inner.as_mut().get_unchecked_mut() as *mut _) };
        let res = poll_future!(self.read_future, cx, inner.fill_read_buf());
        Poll::Ready(res)
    }

    fn poll_ready_impl(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        set_waker: bool,
    ) -> Poll<Result<(), WsError>> {
        if let Some(mut fut) = self.write_future.take() {
            let res = match fut.as_mut().poll(cx) {
                Poll::Pending => {
                    self.write_future.replace(fut);
                    if set_waker {
                        self.flush_waker.replace(cx.waker().clone());
                    }
                    Poll::Pending
                }
                Poll::Ready(Ok(_)) => Poll::Ready(Ok(())),
                Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            };
            if res.is_ready() {
                self.flush_waker.take();
            }
            res
        } else {
            Poll::Ready(Ok(()))
        }
    }
}

impl<S: AsyncRead + AsyncWrite + 'static> Sink<Message> for CompatWebSocketStream<S> {
    type Error = tungstenite::Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.poll_ready_impl(cx, true)
    }

    fn start_send(mut self: Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
        let inner: &'static mut WebSocketStream<S> =
            unsafe { &mut *(self.inner.as_mut().get_unchecked_mut() as *mut _) };
        match inner.inner.write(item) {
            Ok(()) => Ok(()),
            Err(WsError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(()),
            Err(e) => Err(e),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        std::task::ready!(self.as_mut().poll_ready_impl(cx, true))?;

        if !self.flushed {
            let inner: &'static mut WebSocketStream<S> =
                unsafe { &mut *(self.inner.as_mut().get_unchecked_mut() as *mut _) };
            let res = match poll_future_would_block!(
                self.write_future,
                cx,
                inner.flush_write_buf(),
                inner.inner.flush()
            ) {
                Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
                Poll::Ready(Err(WsError::ConnectionClosed)) => Poll::Ready(Ok(())),
                Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
                Poll::Pending => Poll::Pending,
            };
            if let Poll::Ready(Ok(())) = res {
                let inner: &'static mut WebSocketStream<S> =
                    unsafe { &mut *(self.inner.as_mut().get_unchecked_mut() as *mut _) };
                self.flushed = true;
                self.write_future = Some(Box::pin(inner.flush_write_buf()));
                std::task::ready!(self.as_mut().poll_ready_impl(cx, true))?;
            }
            res
        } else {
            self.flushed = false;
            Poll::Ready(Ok(()))
        }
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let inner: &'static mut WebSocketStream<S> =
            unsafe { &mut *(self.inner.as_mut().get_unchecked_mut() as *mut _) };
        match inner.inner.close(None) {
            Ok(()) => {}
            Err(WsError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                let flushed = std::task::ready!(self.as_mut().poll_flush_impl(cx))?;
                if flushed == 0 {
                    std::task::ready!(self.as_mut().poll_fill_impl(cx))?;
                }
                return Poll::Pending;
            }
            Err(WsError::ConnectionClosed) => {}
            Err(e) => return Poll::Ready(Err(e)),
        }
        self.poll_flush(cx)
    }
}

impl<S: AsyncRead + AsyncWrite + 'static> Stream for CompatWebSocketStream<S> {
    type Item = Result<Message, WsError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let inner: &'static mut WebSocketStream<S> =
            unsafe { &mut *(self.inner.as_mut().get_unchecked_mut() as *mut _) };
        let res = match poll_future_would_block!(
            self.read_future,
            cx,
            inner.fill_read_buf(),
            inner.inner.read()
        ) {
            Poll::Ready(Ok(msg)) => Poll::Ready(Some(Ok(msg))),
            Poll::Ready(Err(WsError::ConnectionClosed | WsError::AlreadyClosed)) => {
                Poll::Ready(None)
            }
            Poll::Ready(Err(e)) => Poll::Ready(Some(Err(e))),
            Poll::Pending => Poll::Pending,
        };
        if self.write_future.is_some() {
            let waker = self.flush_waker.as_ref().unwrap_or_else(|| Waker::noop());
            let waker_array = WakerArray([cx.waker().clone(), waker.clone()]);
            let waker = Waker::from(Arc::new(waker_array));
            let mut cx = Context::from_waker(&waker);
            let ready_res = self.as_mut().poll_ready_impl(&mut cx, false);
            if let Poll::Ready(Err(e)) = ready_res
                && let Poll::Ready(Some(Ok(_))) = &res
            {
                return Poll::Ready(Some(Err(e)));
            }
        }
        res
    }
}

struct WakerArray<const N: usize>([Waker; N]);

impl<const N: usize> Wake for WakerArray<N> {
    fn wake(self: Arc<Self>) {
        self.0.iter().for_each(|w| w.wake_by_ref());
    }
}
