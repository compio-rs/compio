use std::{
    io,
    sync::Arc,
    task::{Context, Poll},
};

use bytes::Bytes;
use compio_buf::{BufResult, IoBuf};
use compio_io::AsyncWrite;
use futures_util::{future::poll_fn, ready};
use quinn_proto::{ClosedStream, FinishError, StreamId, VarInt, Written};
use thiserror::Error;

use crate::{ConnectionError, ConnectionInner, StoppedError};

/// A stream that can only be used to send data.
///
/// If dropped, streams that haven't been explicitly [`reset()`] will be
/// implicitly [`finish()`]ed, continuing to (re)transmit previously written
/// data until it has been fully acknowledged or the connection is closed.
///
/// # Cancellation
///
/// A `write` method is said to be *cancel-safe* when dropping its future before
/// the future becomes ready will always result in no data being written to the
/// stream. This is true of methods which succeed immediately when any progress
/// is made, and is not true of methods which might need to perform multiple
/// writes internally before succeeding. Each `write` method documents whether
/// it is cancel-safe.
///
/// [`reset()`]: SendStream::reset
/// [`finish()`]: SendStream::finish
#[derive(Debug)]
pub struct SendStream {
    conn: Arc<ConnectionInner>,
    stream: StreamId,
    is_0rtt: bool,
}

impl SendStream {
    pub(crate) fn new(conn: Arc<ConnectionInner>, stream: StreamId, is_0rtt: bool) -> Self {
        Self {
            conn,
            stream,
            is_0rtt,
        }
    }

    /// Get the identity of this stream
    pub fn id(&self) -> StreamId {
        self.stream
    }

    /// Notify the peer that no more data will ever be written to this stream.
    ///
    /// It is an error to write to a stream after `finish()`ing it. [`reset()`]
    /// may still be called after `finish` to abandon transmission of any stream
    /// data that might still be buffered.
    ///
    /// To wait for the peer to receive all buffered stream data, see
    /// [`stopped()`].
    ///
    /// May fail if [`finish()`] or  [`reset()`] was previously called.This
    /// error is harmless and serves only to indicate that the caller may have
    /// incorrect assumptions about the stream's state.
    ///
    /// [`reset()`]: Self::reset
    /// [`stopped()`]: Self::stopped
    /// [`finish()`]: Self::finish
    pub fn finish(&mut self) -> Result<(), ClosedStream> {
        let mut state = self.conn.state();
        match state.conn.send_stream(self.stream).finish() {
            Ok(()) => {
                state.wake();
                Ok(())
            }
            Err(FinishError::ClosedStream) => Err(ClosedStream::new()),
            // Harmless. If the application needs to know about stopped streams at this point,
            // it should call `stopped`.
            Err(FinishError::Stopped(_)) => Ok(()),
        }
    }

    /// Close the stream immediately.
    ///
    /// No new data can be written after calling this method. Locally buffered
    /// data is dropped, and previously transmitted data will no longer be
    /// retransmitted if lost. If an attempt has already been made to finish
    /// the stream, the peer may still receive all written data.
    ///
    /// May fail if [`finish()`](Self::finish) or [`reset()`](Self::reset) was
    /// previously called. This error is harmless and serves only to
    /// indicate that the caller may have incorrect assumptions about the
    /// stream's state.
    pub fn reset(&mut self, error_code: VarInt) -> Result<(), ClosedStream> {
        let mut state = self.conn.state();
        if self.is_0rtt && !state.check_0rtt() {
            return Ok(());
        }
        state.conn.send_stream(self.stream).reset(error_code)?;
        state.wake();
        Ok(())
    }

    /// Set the priority of the stream.
    ///
    /// Every stream has an initial priority of 0. Locally buffered data
    /// from streams with higher priority will be transmitted before data
    /// from streams with lower priority. Changing the priority of a stream
    /// with pending data may only take effect after that data has been
    /// transmitted. Using many different priority levels per connection may
    /// have a negative impact on performance.
    pub fn set_priority(&self, priority: i32) -> Result<(), ClosedStream> {
        self.conn
            .state()
            .conn
            .send_stream(self.stream)
            .set_priority(priority)
    }

    /// Get the priority of the stream
    pub fn priority(&self) -> Result<i32, ClosedStream> {
        self.conn.state().conn.send_stream(self.stream).priority()
    }

    /// Completes when the peer stops the stream or reads the stream to
    /// completion.
    ///
    /// Yields `Some` with the stop error code if the peer stops the stream.
    /// Yields `None` if the local side [`finish()`](Self::finish)es the stream
    /// and then the peer acknowledges receipt of all stream data (although not
    /// necessarily the processing of it), after which the peer closing the
    /// stream is no longer meaningful.
    ///
    /// For a variety of reasons, the peer may not send acknowledgements
    /// immediately upon receiving data. As such, relying on `stopped` to
    /// know when the peer has read a stream to completion may introduce
    /// more latency than using an application-level response of some sort.
    pub async fn stopped(&mut self) -> Result<Option<VarInt>, StoppedError> {
        poll_fn(|cx| {
            let mut state = self.conn.state();
            if self.is_0rtt && !state.check_0rtt() {
                return Poll::Ready(Err(StoppedError::ZeroRttRejected));
            }
            match state.conn.send_stream(self.stream).stopped() {
                Err(_) => Poll::Ready(Ok(None)),
                Ok(Some(error_code)) => Poll::Ready(Ok(Some(error_code))),
                Ok(None) => {
                    if let Some(e) = &state.error {
                        return Poll::Ready(Err(e.clone().into()));
                    }
                    state.stopped.insert(self.stream, cx.waker().clone());
                    Poll::Pending
                }
            }
        })
        .await
    }

    fn execute_poll_write<F, R>(&mut self, cx: &mut Context, f: F) -> Poll<Result<R, WriteError>>
    where
        F: FnOnce(quinn_proto::SendStream) -> Result<R, quinn_proto::WriteError>,
    {
        let mut state = self.conn.try_state()?;
        if self.is_0rtt && !state.check_0rtt() {
            return Poll::Ready(Err(WriteError::ZeroRttRejected));
        }
        match f(state.conn.send_stream(self.stream)) {
            Ok(r) => {
                state.wake();
                Poll::Ready(Ok(r))
            }
            Err(e) => match e.try_into() {
                Ok(e) => Poll::Ready(Err(e)),
                Err(()) => {
                    state.writable.insert(self.stream, cx.waker().clone());
                    Poll::Pending
                }
            },
        }
    }

    /// Write bytes to the stream.
    ///
    /// Yields the number of bytes written on success. Congestion and flow
    /// control may cause this to be shorter than `buf.len()`, indicating
    /// that only a prefix of `buf` was written.
    ///
    /// This operation is cancel-safe.
    pub async fn write(&mut self, buf: &[u8]) -> Result<usize, WriteError> {
        poll_fn(|cx| self.execute_poll_write(cx, |mut stream| stream.write(buf))).await
    }

    /// Convenience method to write an entire buffer to the stream.
    ///
    /// This operation is *not* cancel-safe.
    pub async fn write_all(&mut self, buf: &[u8]) -> Result<(), WriteError> {
        let mut count = 0;
        poll_fn(|cx| {
            loop {
                if count == buf.len() {
                    return Poll::Ready(Ok(()));
                }
                let n =
                    ready!(self.execute_poll_write(cx, |mut stream| stream.write(&buf[count..])))?;
                count += n;
            }
        })
        .await
    }

    /// Write chunks to the stream.
    ///
    /// Yields the number of bytes and chunks written on success.
    /// Congestion and flow control may cause this to be shorter than
    /// `buf.len()`, indicating that only a prefix of `bufs` was written.
    ///
    /// This operation is cancel-safe.
    pub async fn write_chunks(&mut self, bufs: &mut [Bytes]) -> Result<Written, WriteError> {
        poll_fn(|cx| self.execute_poll_write(cx, |mut stream| stream.write_chunks(bufs))).await
    }

    /// Convenience method to write an entire list of chunks to the stream.
    ///
    /// This operation is *not* cancel-safe.
    pub async fn write_all_chunks(&mut self, bufs: &mut [Bytes]) -> Result<(), WriteError> {
        let mut chunks = 0;
        poll_fn(|cx| {
            loop {
                if chunks == bufs.len() {
                    return Poll::Ready(Ok(()));
                }
                let written = ready!(self.execute_poll_write(cx, |mut stream| {
                    stream.write_chunks(&mut bufs[chunks..])
                }))?;
                chunks += written.chunks;
            }
        })
        .await
    }
}

impl Drop for SendStream {
    fn drop(&mut self) {
        let mut state = self.conn.state();

        // clean up any previously registered wakers
        state.stopped.remove(&self.stream);
        state.writable.remove(&self.stream);

        if state.error.is_some() || (self.is_0rtt && !state.check_0rtt()) {
            return;
        }
        match state.conn.send_stream(self.stream).finish() {
            Ok(()) => state.wake(),
            Err(FinishError::Stopped(reason)) => {
                if state.conn.send_stream(self.stream).reset(reason).is_ok() {
                    state.wake();
                }
            }
            // Already finished or reset, which is fine.
            Err(FinishError::ClosedStream) => {}
        }
    }
}

/// Errors that arise from writing to a stream
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum WriteError {
    /// The peer is no longer accepting data on this stream
    ///
    /// Carries an application-defined error code.
    #[error("sending stopped by peer: error {0}")]
    Stopped(VarInt),
    /// The connection was lost
    #[error("connection lost")]
    ConnectionLost(#[from] ConnectionError),
    /// The stream has already been finished or reset
    #[error("closed stream")]
    ClosedStream,
    /// This was a 0-RTT stream and the server rejected it
    ///
    /// Can only occur on clients for 0-RTT streams, which can be opened using
    /// [`Connecting::into_0rtt()`].
    ///
    /// [`Connecting::into_0rtt()`]: crate::Connecting::into_0rtt()
    #[error("0-RTT rejected")]
    ZeroRttRejected,
    /// Error when the stream is not ready, because it is still sending
    /// data from a previous call
    #[cfg(feature = "h3")]
    #[error("stream not ready")]
    NotReady,
}

impl TryFrom<quinn_proto::WriteError> for WriteError {
    type Error = ();

    fn try_from(value: quinn_proto::WriteError) -> Result<Self, Self::Error> {
        use quinn_proto::WriteError::*;
        match value {
            Stopped(e) => Ok(Self::Stopped(e)),
            ClosedStream => Ok(Self::ClosedStream),
            Blocked => Err(()),
        }
    }
}

impl From<StoppedError> for WriteError {
    fn from(x: StoppedError) -> Self {
        match x {
            StoppedError::ConnectionLost(e) => Self::ConnectionLost(e),
            StoppedError::ZeroRttRejected => Self::ZeroRttRejected,
        }
    }
}

impl From<WriteError> for io::Error {
    fn from(x: WriteError) -> Self {
        use WriteError::*;
        let kind = match x {
            Stopped(_) | ZeroRttRejected => io::ErrorKind::ConnectionReset,
            ConnectionLost(_) | ClosedStream => io::ErrorKind::NotConnected,
            #[cfg(feature = "h3")]
            NotReady => io::ErrorKind::Other,
        };
        Self::new(kind, x)
    }
}

impl AsyncWrite for SendStream {
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        let res = self.write(buf.as_slice()).await.map_err(Into::into);
        BufResult(res, buf)
    }

    async fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }

    async fn shutdown(&mut self) -> io::Result<()> {
        self.finish()?;
        Ok(())
    }
}

#[cfg(feature = "io-compat")]
impl futures_util::AsyncWrite for SendStream {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.get_mut()
            .execute_poll_write(cx, |mut stream| stream.write(buf))
            .map_err(Into::into)
    }

    fn poll_flush(self: std::pin::Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: std::pin::Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.get_mut().finish()?;
        Poll::Ready(Ok(()))
    }
}

#[cfg(feature = "h3")]
pub(crate) mod h3_impl {
    use bytes::Buf;
    use h3::quic::{self, Error, WriteBuf};

    use super::*;

    impl Error for WriteError {
        fn is_timeout(&self) -> bool {
            matches!(self, Self::ConnectionLost(ConnectionError::TimedOut))
        }

        fn err_code(&self) -> Option<u64> {
            match self {
                Self::ConnectionLost(ConnectionError::ApplicationClosed(
                    quinn_proto::ApplicationClose { error_code, .. },
                ))
                | Self::Stopped(error_code) => Some(error_code.into_inner()),
                _ => None,
            }
        }
    }

    /// A wrapper around `SendStream` that implements `quic::SendStream` and
    /// `quic::SendStreamUnframed`.
    pub struct SendStream<B> {
        inner: super::SendStream,
        buf: Option<WriteBuf<B>>,
    }

    impl<B> SendStream<B> {
        pub(crate) fn new(conn: Arc<ConnectionInner>, stream: StreamId, is_0rtt: bool) -> Self {
            Self {
                inner: super::SendStream::new(conn, stream, is_0rtt),
                buf: None,
            }
        }
    }

    impl<B> quic::SendStream<B> for SendStream<B>
    where
        B: Buf,
    {
        type Error = WriteError;

        fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            if let Some(data) = &mut self.buf {
                while data.has_remaining() {
                    let n = ready!(
                        self.inner
                            .execute_poll_write(cx, |mut stream| stream.write(data.chunk()))
                    )?;
                    data.advance(n);
                }
            }
            self.buf = None;
            Poll::Ready(Ok(()))
        }

        fn send_data<T: Into<WriteBuf<B>>>(&mut self, data: T) -> Result<(), Self::Error> {
            if self.buf.is_some() {
                return Err(WriteError::NotReady);
            }
            self.buf = Some(data.into());
            Ok(())
        }

        fn poll_finish(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(self.inner.finish().map_err(|_| WriteError::ClosedStream))
        }

        fn reset(&mut self, reset_code: u64) {
            self.inner
                .reset(reset_code.try_into().unwrap_or(VarInt::MAX))
                .ok();
        }

        fn send_id(&self) -> quic::StreamId {
            self.inner.stream.0.try_into().unwrap()
        }
    }

    impl<B> quic::SendStreamUnframed<B> for SendStream<B>
    where
        B: Buf,
    {
        fn poll_send<D: Buf>(
            &mut self,
            cx: &mut Context<'_>,
            buf: &mut D,
        ) -> Poll<Result<usize, Self::Error>> {
            // This signifies a bug in implementation
            debug_assert!(
                self.buf.is_some(),
                "poll_send called while send stream is not ready"
            );

            let n = ready!(
                self.inner
                    .execute_poll_write(cx, |mut stream| stream.write(buf.chunk()))
            )?;
            buf.advance(n);
            Poll::Ready(Ok(n))
        }
    }
}
