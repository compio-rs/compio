use std::{
    ops::Deref,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, Wake, Waker, ready},
};

use compio_io::{AsyncRead, AsyncWrite, compat::SyncStream};
use futures_util::{Sink, Stream};
use pin_project_lite::pin_project;
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

pin_project! {
    /// A [`futures_util`] compatible WebSocket stream.
    pub struct CompatWebSocketStream<S> {
        #[pin]
        inner: WebSocket<SyncStream<S>>,
        read_future: Option<PinBoxFuture<Result<usize, std::io::Error>>>,
        write_future: Option<PinBoxFuture<Result<usize, std::io::Error>>>,
        ready_waker: Option<Waker>,
        flush_waker: Option<Waker>,
        close_waker: Option<Waker>,
        read_waker: Option<Waker>,
        flushing: Flushing,
        closing: Closing,
        reading: Reading,
        // This is a self-referential struct, so we need to prevent it from being `Unpin`.
        #[pin]
    }
}

impl<S> CompatWebSocketStream<S> {
    pub(super) fn new(stream: WebSocket<SyncStream<S>>) -> Self {
        Self {
            inner: stream,
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
    fn poll_flush_write_buf(self: Pin<&mut Self>) -> Poll<Result<usize, WsError>> {
        let this = self.project();
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
            unsafe { extend_lifetime(this.inner.get_mut().get_mut()) };
        let arr = WakerArray([
            this.ready_waker.as_ref().cloned(),
            this.flush_waker.as_ref().cloned(),
            this.close_waker.as_ref().cloned(),
            this.read_waker.as_ref().cloned(),
        ]);
        let waker = Waker::from(Arc::new(arr));
        let cx = &mut Context::from_waker(&waker);
        let res = poll_future!(this.write_future, cx, inner.flush_write_buf());
        Poll::Ready(res.map_err(WsError::Io))
    }

    fn poll_fill_read_buf(self: Pin<&mut Self>) -> Poll<Result<usize, WsError>> {
        let this = self.project();
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
            unsafe { extend_lifetime(this.inner.get_mut().get_mut()) };
        let arr = WakerArray([
            this.close_waker.as_ref().cloned(),
            this.read_waker.as_ref().cloned(),
        ]);
        let waker = Waker::from(Arc::new(arr));
        let cx = &mut Context::from_waker(&waker);
        let res = poll_future!(this.read_future, cx, inner.fill_read_buf());
        Poll::Ready(res.map_err(WsError::Io))
    }

    fn poll_flush_impl(mut self: Pin<&mut Self>) -> Poll<Result<(), WsError>> {
        loop {
            let mut this = self.as_mut().project();
            match this.flushing {
                Flushing::None => {
                    *this.flushing = match this.inner.flush() {
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
                    *self.as_mut().project().flushing = Flushing::None
                }
                Flushing::Flushed => {
                    ready!(self.as_mut().poll_flush_write_buf())?;
                    let this = self.as_mut().project();
                    *this.flushing = Flushing::None;
                    this.flush_waker.take();
                    return Poll::Ready(Ok(()));
                }
            }
        }
    }
}

fn replace_waker(waker_slot: &mut Option<Waker>, waker: &Waker) {
    if !waker_slot.as_ref().is_some_and(|w| w.will_wake(waker)) {
        waker_slot.replace(waker.clone());
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin + 'static> Sink<Message> for CompatWebSocketStream<S>
where
    for<'a> &'a S: AsyncRead + AsyncWrite,
{
    type Error = tungstenite::Error;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        if self.write_future.is_some() {
            replace_waker(self.as_mut().project().ready_waker, cx.waker());
            ready!(self.as_mut().poll_flush_write_buf())?;
            self.as_mut().project().ready_waker.take();
        }
        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
        match self.project().inner.write(item) {
            Ok(()) => Ok(()),
            Err(WsError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(()),
            Err(e) => Err(e),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        replace_waker(self.as_mut().project().flush_waker, cx.waker());
        self.poll_flush_impl()
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        replace_waker(self.as_mut().project().close_waker, cx.waker());
        loop {
            let mut this = self.as_mut().project();
            match this.closing {
                Closing::None => {
                    *this.closing = match this.inner.close(None) {
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
                    *self.as_mut().project().closing = if flushed == 0 {
                        Closing::WouldBlockFill
                    } else {
                        Closing::None
                    }
                }
                Closing::WouldBlockFill => {
                    ready!(self.as_mut().poll_fill_read_buf())?;
                    *self.as_mut().project().closing = Closing::None;
                }
                Closing::Closed => {
                    ready!(self.as_mut().poll_flush_impl())?;
                    let this = self.as_mut().project();
                    *this.closing = Closing::None;
                    this.close_waker.take();
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
        replace_waker(self.as_mut().project().read_waker, cx.waker());
        loop {
            let mut this = self.as_mut().project();
            match std::mem::replace(this.reading, Reading::None) {
                Reading::None => {
                    *this.reading = match this.inner.read() {
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
                    self.as_mut().project().read_waker.take();
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
