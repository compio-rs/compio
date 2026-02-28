use std::{
    ops::Deref,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, Wake, Waker, ready},
};

use compio_io::{AsyncRead, AsyncWrite, compat::SyncStream};
use futures_util::{Sink, Stream};
use tungstenite::{Message, WebSocket};

use crate::WsError;

type PinBoxFuture<T> = Pin<Box<dyn Future<Output = T>>>;

enum Flushing {
    None,
    WouldBlock,
    Flushed,
}

enum Closing {
    None,
    WouldBlockFlush,
    WouldBlockFill,
    Closed,
}

enum Reading {
    None,
    AfterRead(Result<Message, WsError>),
    WouldBlock,
}

/// A [`futures_util`] compatible WebSocket stream.
pub struct CompatWebSocketStream<S> {
    inner: Pin<Box<WebSocket<SyncStream<S>>>>,
    read_future: Option<PinBoxFuture<Result<usize, std::io::Error>>>,
    write_future: Option<PinBoxFuture<Result<usize, std::io::Error>>>,
    ready_waker: Option<Waker>,
    flush_waker: Option<Waker>,
    close_waker: Option<Waker>,
    read_waker: Option<Waker>,
    flushing: Flushing,
    closing: Closing,
    reading: Reading,
}

impl<S> CompatWebSocketStream<S> {
    pub(super) fn new(stream: WebSocket<SyncStream<S>>) -> Self {
        Self {
            inner: Box::pin(stream),
            read_future: None,
            write_future: None,
            ready_waker: None,
            flush_waker: None,
            close_waker: None,
            read_waker: None,
            flushing: Flushing::None,
            closing: Closing::None,
            reading: Reading::None,
        }
    }
}

impl<S> Deref for CompatWebSocketStream<S> {
    type Target = WebSocket<SyncStream<S>>;

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

unsafe fn extend_lifetime<T>(t: &mut T) -> &'static mut T {
    unsafe { &mut *(t as *mut T) }
}

impl<S: AsyncRead + AsyncWrite + Unpin + 'static> CompatWebSocketStream<S>
where
    for<'a> &'a S: AsyncRead + AsyncWrite,
{
    fn poll_flush_write_buf(mut self: Pin<&mut Self>) -> Poll<Result<usize, WsError>> {
        // SAFETY:
        // - The future won't live longer than the stream.
        // - The stream is `Unpin`, and is internally mutable.
        // - The future only accesses the corresponding buffer and fields.
        //   - No access overlap between the futures.
        // - The future is polled immediately after creation, so it takes the ownership
        //   of the inner buffer.
        // - The sync methods of `SyncStream` check if the inner buffer is already
        //   borrowed, and returns `WouldBlock` if it is.
        let inner: &'static mut SyncStream<S> =
            unsafe { extend_lifetime(self.inner.as_mut().get_mut().get_mut()) };
        let arr = WakerArray([
            self.ready_waker.as_ref().cloned(),
            self.flush_waker.as_ref().cloned(),
            self.close_waker.as_ref().cloned(),
            self.read_waker.as_ref().cloned(),
        ]);
        let waker = Waker::from(Arc::new(arr));
        let cx = &mut Context::from_waker(&waker);
        let res = poll_future!(self.write_future, cx, inner.flush_write_buf());
        Poll::Ready(res.map_err(WsError::Io))
    }

    fn poll_fill_read_buf(mut self: Pin<&mut Self>) -> Poll<Result<usize, WsError>> {
        // SAFETY:
        // - The future won't live longer than the stream.
        // - The stream is `Unpin`, and is internally mutable.
        // - The future only accesses the corresponding buffer and fields.
        //   - No access overlap between the futures.
        // - The future is polled immediately after creation, so it takes the ownership
        //   of the inner buffer.
        // - The sync methods of `SyncStream` check if the inner buffer is already
        //   borrowed, and returns `WouldBlock` if it is.
        let inner: &'static mut SyncStream<S> =
            unsafe { extend_lifetime(self.inner.as_mut().get_mut().get_mut()) };
        let arr = WakerArray([
            self.close_waker.as_ref().cloned(),
            self.read_waker.as_ref().cloned(),
        ]);
        let waker = Waker::from(Arc::new(arr));
        let cx = &mut Context::from_waker(&waker);
        let res = poll_future!(self.read_future, cx, inner.fill_read_buf());
        Poll::Ready(res.map_err(WsError::Io))
    }

    fn poll_flush_impl(mut self: Pin<&mut Self>) -> Poll<Result<(), WsError>> {
        loop {
            match self.flushing {
                Flushing::None => {
                    self.flushing = match self.inner.flush() {
                        Ok(()) => Flushing::Flushed,
                        Err(WsError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            Flushing::WouldBlock
                        }
                        Err(WsError::ConnectionClosed) => Flushing::Flushed,
                        Err(e) => return Poll::Ready(Err(e)),
                    }
                }
                Flushing::WouldBlock => {
                    ready!(self.as_mut().poll_flush_write_buf())?;
                    self.flushing = Flushing::None
                }
                Flushing::Flushed => {
                    ready!(self.as_mut().poll_flush_write_buf())?;
                    self.flushing = Flushing::None;
                    self.flush_waker.take();
                    return Poll::Ready(Ok(()));
                }
            }
        }
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin + 'static> Sink<Message> for CompatWebSocketStream<S>
where
    for<'a> &'a S: AsyncRead + AsyncWrite,
{
    type Error = tungstenite::Error;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.write_future.is_some() {
            self.ready_waker.replace(cx.waker().clone());
            ready!(self.as_mut().poll_flush_write_buf())?;
            self.read_waker.take();
        }
        Poll::Ready(Ok(()))
    }

    fn start_send(mut self: Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
        match self.inner.write(item) {
            Ok(()) => Ok(()),
            Err(WsError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(()),
            Err(e) => Err(e),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.flush_waker.replace(cx.waker().clone());
        self.poll_flush_impl()
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.close_waker.replace(cx.waker().clone());
        loop {
            match self.closing {
                Closing::None => {
                    self.closing = match self.inner.close(None) {
                        Ok(()) => Closing::Closed,
                        Err(WsError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            Closing::WouldBlockFlush
                        }
                        Err(WsError::ConnectionClosed) => Closing::Closed,
                        Err(e) => return Poll::Ready(Err(e)),
                    }
                }
                Closing::WouldBlockFlush => {
                    let flushed = ready!(self.as_mut().poll_flush_write_buf())?;
                    self.closing = if flushed == 0 {
                        Closing::WouldBlockFill
                    } else {
                        Closing::None
                    }
                }
                Closing::WouldBlockFill => {
                    ready!(self.as_mut().poll_fill_read_buf())?;
                    self.closing = Closing::None;
                }
                Closing::Closed => {
                    self.close_waker.take();
                    ready!(self.as_mut().poll_flush_impl())?;
                    self.closing = Closing::None;
                    return Poll::Ready(Ok(()));
                }
            }
        }
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin + 'static> Stream for CompatWebSocketStream<S>
where
    for<'a> &'a S: AsyncRead + AsyncWrite,
{
    type Item = Result<Message, WsError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.read_waker.replace(cx.waker().clone());
        loop {
            match std::mem::replace(&mut self.reading, Reading::None) {
                Reading::None => {
                    self.reading = match self.inner.read() {
                        Ok(msg) => Reading::AfterRead(Ok(msg)),
                        Err(WsError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            Reading::WouldBlock
                        }
                        Err(WsError::AlreadyClosed | WsError::ConnectionClosed) => {
                            return Poll::Ready(None);
                        }
                        Err(e) => Reading::AfterRead(Err(e)),
                    }
                }
                Reading::WouldBlock => {
                    ready!(self.as_mut().poll_fill_read_buf())?;
                }
                Reading::AfterRead(res) => {
                    let res = match self.as_mut().poll_flush_impl() {
                        Poll::Pending => res,
                        Poll::Ready(Ok(())) => res,
                        Poll::Ready(Err(e)) => {
                            if let Err(ori_e) = res {
                                Err(ori_e)
                            } else {
                                Err(e)
                            }
                        }
                    };
                    self.read_waker.take();
                    return Poll::Ready(Some(res));
                }
            }
        }
    }
}

struct WakerArray<const N: usize>([Option<Waker>; N]);

impl<const N: usize> Wake for WakerArray<N> {
    fn wake(self: Arc<Self>) {
        self.0.iter().for_each(|w| {
            if let Some(w) = w {
                w.wake_by_ref()
            }
        });
    }
}
