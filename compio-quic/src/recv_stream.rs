use std::{
    collections::BTreeMap,
    io,
    sync::Arc,
    task::{Context, Poll},
};

use bytes::{BufMut, Bytes};
use compio_buf::{BufResult, IoBufMut};
use compio_io::AsyncRead;
use futures_util::future::poll_fn;
use quinn_proto::{Chunk, Chunks, ClosedStream, ConnectionError, ReadableError, StreamId, VarInt};
use thiserror::Error;

use crate::{ConnectionInner, StoppedError};

/// A stream that can only be used to receive data
///
/// `stop(0)` is implicitly called on drop unless:
/// - A variant of [`ReadError`] has been yielded by a read call
/// - [`stop()`] was called explicitly
///
/// # Cancellation
///
/// A `read` method is said to be *cancel-safe* when dropping its future before
/// the future becomes ready cannot lead to loss of stream data. This is true of
/// methods which succeed immediately when any progress is made, and is not true
/// of methods which might need to perform multiple reads internally before
/// succeeding. Each `read` method documents whether it is cancel-safe.
///
/// # Common issues
///
/// ## Data never received on a locally-opened stream
///
/// Peers are not notified of streams until they or a later-numbered stream are
/// used to send data. If a bidirectional stream is locally opened but never
/// used to send, then the peer may never see it. Application protocols should
/// always arrange for the endpoint which will first transmit on a stream to be
/// the endpoint responsible for opening it.
///
/// ## Data never received on a remotely-opened stream
///
/// Verify that the stream you are receiving is the same one that the server is
/// sending on, e.g. by logging the [`id`] of each. Streams are always accepted
/// in the same order as they are created, i.e. ascending order by [`StreamId`].
/// For example, even if a sender first transmits on bidirectional stream 1, the
/// first stream yielded by [`Connection::accept_bi`] on the receiver
/// will be bidirectional stream 0.
///
/// [`stop()`]: RecvStream::stop
/// [`id`]: RecvStream::id
/// [`Connection::accept_bi`]: crate::Connection::accept_bi
#[derive(Debug)]
pub struct RecvStream {
    conn: Arc<ConnectionInner>,
    stream: StreamId,
    is_0rtt: bool,
    all_data_read: bool,
    reset: Option<VarInt>,
}

impl RecvStream {
    pub(crate) fn new(conn: Arc<ConnectionInner>, stream: StreamId, is_0rtt: bool) -> Self {
        Self {
            conn,
            stream,
            is_0rtt,
            all_data_read: false,
            reset: None,
        }
    }

    /// Get the identity of this stream
    pub fn id(&self) -> StreamId {
        self.stream
    }

    /// Check if this stream has been opened during 0-RTT.
    ///
    /// In which case any non-idempotent request should be considered dangerous
    /// at the application level. Because read data is subject to replay
    /// attacks.
    pub fn is_0rtt(&self) -> bool {
        self.is_0rtt
    }

    /// Stop accepting data
    ///
    /// Discards unread data and notifies the peer to stop transmitting. Once
    /// stopped, further attempts to operate on a stream will yield
    /// `ClosedStream` errors.
    pub fn stop(&mut self, error_code: VarInt) -> Result<(), ClosedStream> {
        let mut state = self.conn.state();
        if self.is_0rtt && state.check_0rtt().is_err() {
            return Ok(());
        }
        state.conn.recv_stream(self.stream).stop(error_code)?;
        state.wake();
        self.all_data_read = true;
        Ok(())
    }

    /// Completes when the stream has been reset by the peer or otherwise
    /// closed.
    ///
    /// Yields `Some` with the reset error code when the stream is reset by the
    /// peer. Yields `None` when the stream was previously
    /// [`stop()`](Self::stop)ed, or when the stream was
    /// [`finish()`](crate::SendStream::finish)ed by the peer and all data has
    /// been received, after which it is no longer meaningful for the stream to
    /// be reset.
    ///
    /// This operation is cancel-safe.
    pub async fn stopped(&mut self) -> Result<Option<VarInt>, StoppedError> {
        poll_fn(|cx| {
            let mut state = self.conn.state();

            if self.is_0rtt && state.check_0rtt().is_err() {
                return Poll::Ready(Err(StoppedError::ZeroRttRejected));
            }
            if let Some(code) = self.reset {
                return Poll::Ready(Ok(Some(code)));
            }

            match state.conn.recv_stream(self.stream).received_reset() {
                Err(_) => Poll::Ready(Ok(None)),
                Ok(Some(error_code)) => {
                    // Stream state has just now been freed, so the connection may need to issue new
                    // stream ID flow control credit
                    state.wake();
                    Poll::Ready(Ok(Some(error_code)))
                }
                Ok(None) => {
                    if let Some(e) = &state.error {
                        return Poll::Ready(Err(e.clone().into()));
                    }
                    // Resets always notify readers, since a reset is an immediate read error. We
                    // could introduce a dedicated channel to reduce the risk of spurious wakeups,
                    // but that increased complexity is probably not justified, as an application
                    // that is expecting a reset is not likely to receive large amounts of data.
                    state.readable.insert(self.stream, cx.waker().clone());
                    Poll::Pending
                }
            }
        })
        .await
    }

    /// Handle common logic related to reading out of a receive stream.
    ///
    /// This takes an `FnMut` closure that takes care of the actual reading
    /// process, matching the detailed read semantics for the calling
    /// function with a particular return type. The closure can read from
    /// the passed `&mut Chunks` and has to return the status after reading:
    /// the amount of data read, and the status after the final read call.
    fn execute_poll_read<F, T>(
        &mut self,
        cx: &mut Context,
        ordered: bool,
        mut read_fn: F,
    ) -> Poll<Result<Option<T>, ReadError>>
    where
        F: FnMut(&mut Chunks) -> ReadStatus<T>,
    {
        use quinn_proto::ReadError::*;

        if self.all_data_read {
            return Poll::Ready(Ok(None));
        }

        let mut state = self.conn.state();
        if self.is_0rtt {
            state
                .check_0rtt()
                .map_err(|()| ReadError::ZeroRttRejected)?;
        }

        // If we stored an error during a previous call, return it now. This can happen
        // if a `read_fn` both wants to return data and also returns an error in
        // its final stream status.
        let status = match self.reset {
            Some(code) => ReadStatus::Failed(None, Reset(code)),
            None => {
                let mut recv = state.conn.recv_stream(self.stream);
                let mut chunks = recv.read(ordered)?;
                let status = read_fn(&mut chunks);
                if chunks.finalize().should_transmit() {
                    state.wake();
                }
                status
            }
        };

        match status {
            ReadStatus::Readable(read) => Poll::Ready(Ok(Some(read))),
            ReadStatus::Finished(read) => {
                self.all_data_read = true;
                Poll::Ready(Ok(read))
            }
            ReadStatus::Failed(read, Blocked) => match read {
                Some(val) => Poll::Ready(Ok(Some(val))),
                None => {
                    if let Some(error) = &state.error {
                        return Poll::Ready(Err(error.clone().into()));
                    }
                    state.readable.insert(self.stream, cx.waker().clone());
                    Poll::Pending
                }
            },
            ReadStatus::Failed(read, Reset(error_code)) => match read {
                None => {
                    self.all_data_read = true;
                    self.reset = Some(error_code);
                    Poll::Ready(Err(ReadError::Reset(error_code)))
                }
                done => {
                    self.reset = Some(error_code);
                    Poll::Ready(Ok(done))
                }
            },
        }
    }

    fn poll_read(
        &mut self,
        cx: &mut Context,
        mut buf: impl BufMut,
    ) -> Poll<Result<Option<usize>, ReadError>> {
        if !buf.has_remaining_mut() {
            return Poll::Ready(Ok(Some(0)));
        }

        self.execute_poll_read(cx, true, |chunks| {
            let mut read = 0;
            loop {
                if !buf.has_remaining_mut() {
                    // We know `read` is `true` because `buf.remaining()` was not 0 before
                    return ReadStatus::Readable(read);
                }

                match chunks.next(buf.remaining_mut()) {
                    Ok(Some(chunk)) => {
                        read += chunk.bytes.len();
                        buf.put(chunk.bytes);
                    }
                    res => {
                        return (if read == 0 { None } else { Some(read) }, res.err()).into();
                    }
                }
            }
        })
    }

    /// Read data contiguously from the stream.
    ///
    /// Yields the number of bytes read into `buf` on success, or `None` if the
    /// stream was finished.
    ///
    /// This operation is cancel-safe.
    pub async fn read(&mut self, mut buf: impl BufMut) -> Result<Option<usize>, ReadError> {
        poll_fn(|cx| self.poll_read(cx, &mut buf)).await
    }

    /// Read the next segment of data.
    ///
    /// Yields `None` if the stream was finished. Otherwise, yields a segment of
    /// data and its offset in the stream. If `ordered` is `true`, the chunk's
    /// offset will be immediately after the last data yielded by
    /// [`read()`](Self::read) or [`read_chunk()`](Self::read_chunk). If
    /// `ordered` is `false`, segments may be received in any order, and the
    /// `Chunk`'s `offset` field can be used to determine ordering in the
    /// caller. Unordered reads are less prone to head-of-line blocking within a
    /// stream, but require the application to manage reassembling the original
    /// data.
    ///
    /// Slightly more efficient than `read` due to not copying. Chunk boundaries
    /// do not correspond to peer writes, and hence cannot be used as framing.
    ///
    /// This operation is cancel-safe.
    pub async fn read_chunk(
        &mut self,
        max_length: usize,
        ordered: bool,
    ) -> Result<Option<Chunk>, ReadError> {
        poll_fn(|cx| {
            self.execute_poll_read(cx, ordered, |chunks| match chunks.next(max_length) {
                Ok(Some(chunk)) => ReadStatus::Readable(chunk),
                res => (None, res.err()).into(),
            })
        })
        .await
    }

    /// Read the next segments of data.
    ///
    /// Fills `bufs` with the segments of data beginning immediately after the
    /// last data yielded by `read` or `read_chunk`, or `None` if the stream was
    /// finished.
    ///
    /// Slightly more efficient than `read` due to not copying. Chunk boundaries
    /// do not correspond to peer writes, and hence cannot be used as framing.
    ///
    /// This operation is cancel-safe.
    pub async fn read_chunks(&mut self, bufs: &mut [Bytes]) -> Result<Option<usize>, ReadError> {
        if bufs.is_empty() {
            return Ok(Some(0));
        }

        poll_fn(|cx| {
            self.execute_poll_read(cx, true, |chunks| {
                let mut read = 0;
                loop {
                    if read >= bufs.len() {
                        // We know `read > 0` because `bufs` cannot be empty here
                        return ReadStatus::Readable(read);
                    }

                    match chunks.next(usize::MAX) {
                        Ok(Some(chunk)) => {
                            bufs[read] = chunk.bytes;
                            read += 1;
                        }
                        res => {
                            return (if read == 0 { None } else { Some(read) }, res.err()).into();
                        }
                    }
                }
            })
        })
        .await
    }

    /// Convenience method to read all remaining data into a buffer.
    ///
    /// Uses unordered reads to be more efficient than using [`AsyncRead`]. If
    /// unordered reads have already been made, the resulting buffer may have
    /// gaps containing zero.
    ///
    /// Depending on [`BufMut`] implementation, this method may fail with
    /// [`ReadError::BufferTooShort`] if the buffer is not large enough to
    /// hold the entire stream. For example when using a `&mut [u8]` it will
    /// never receive bytes more than the length of the slice, but when using a
    /// `&mut Vec<u8>` it will allocate more memory as needed.
    ///
    /// This operation is *not* cancel-safe.
    pub async fn read_to_end(&mut self, mut buf: impl BufMut) -> Result<usize, ReadError> {
        let mut start = u64::MAX;
        let mut end = 0;
        let mut chunks = BTreeMap::new();
        loop {
            let Some(chunk) = self.read_chunk(usize::MAX, false).await? else {
                break;
            };
            start = start.min(chunk.offset);
            end = end.max(chunk.offset + chunk.bytes.len() as u64);
            if end - start > buf.remaining_mut() as u64 {
                return Err(ReadError::BufferTooShort);
            }
            chunks.insert(chunk.offset, chunk.bytes);
        }
        let mut last = 0;
        for (offset, bytes) in chunks {
            let offset = (offset - start) as usize;
            if offset > last {
                buf.put_bytes(0, offset - last);
            }
            last = offset + bytes.len();
            buf.put(bytes);
        }
        Ok((end - start) as usize)
    }
}

impl Drop for RecvStream {
    fn drop(&mut self) {
        let mut state = self.conn.state();

        // clean up any previously registered wakers
        state.readable.remove(&self.stream);

        if state.error.is_some() || (self.is_0rtt && state.check_0rtt().is_err()) {
            return;
        }
        if !self.all_data_read {
            // Ignore ClosedStream errors
            let _ = state.conn.recv_stream(self.stream).stop(0u32.into());
            state.wake();
        }
    }
}

enum ReadStatus<T> {
    Readable(T),
    Finished(Option<T>),
    Failed(Option<T>, quinn_proto::ReadError),
}

impl<T> From<(Option<T>, Option<quinn_proto::ReadError>)> for ReadStatus<T> {
    fn from(status: (Option<T>, Option<quinn_proto::ReadError>)) -> Self {
        match status {
            (read, None) => Self::Finished(read),
            (read, Some(e)) => Self::Failed(read, e),
        }
    }
}

/// Errors that arise from reading from a stream.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// The peer abandoned transmitting data on this stream.
    ///
    /// Carries an application-defined error code.
    #[error("stream reset by peer: error {0}")]
    Reset(VarInt),
    /// The connection was lost.
    #[error("connection lost")]
    ConnectionLost(#[from] ConnectionError),
    /// The stream has already been stopped, finished, or reset.
    #[error("closed stream")]
    ClosedStream,
    /// Attempted an ordered read following an unordered read.
    ///
    /// Performing an unordered read allows discontinuities to arise in the
    /// receive buffer of a stream which cannot be recovered, making further
    /// ordered reads impossible.
    #[error("ordered read after unordered read")]
    IllegalOrderedRead,
    /// This was a 0-RTT stream and the server rejected it.
    ///
    /// Can only occur on clients for 0-RTT streams, which can be opened using
    /// [`Connecting::into_0rtt()`].
    ///
    /// [`Connecting::into_0rtt()`]: crate::Connecting::into_0rtt()
    #[error("0-RTT rejected")]
    ZeroRttRejected,
    /// The stream is larger than the user-supplied buffer capacity.
    ///
    /// Can only occur when using [`read_to_end()`](RecvStream::read_to_end).
    #[error("buffer too short")]
    BufferTooShort,
}

impl From<ReadableError> for ReadError {
    fn from(e: ReadableError) -> Self {
        match e {
            ReadableError::ClosedStream => Self::ClosedStream,
            ReadableError::IllegalOrderedRead => Self::IllegalOrderedRead,
        }
    }
}

impl From<StoppedError> for ReadError {
    fn from(e: StoppedError) -> Self {
        match e {
            StoppedError::ConnectionLost(e) => Self::ConnectionLost(e),
            StoppedError::ZeroRttRejected => Self::ZeroRttRejected,
        }
    }
}

impl From<ReadError> for io::Error {
    fn from(x: ReadError) -> Self {
        use self::ReadError::*;
        let kind = match x {
            Reset { .. } | ZeroRttRejected => io::ErrorKind::ConnectionReset,
            ConnectionLost(_) | ClosedStream => io::ErrorKind::NotConnected,
            IllegalOrderedRead | BufferTooShort => io::ErrorKind::InvalidInput,
        };
        Self::new(kind, x)
    }
}

impl AsyncRead for RecvStream {
    async fn read<B: IoBufMut>(&mut self, mut buf: B) -> BufResult<usize, B> {
        let res = self
            .read(buf.as_mut_slice())
            .await
            .map(|n| {
                let n = n.unwrap_or_default();
                unsafe { buf.set_buf_init(n) }
                n
            })
            .map_err(Into::into);
        BufResult(res, buf)
    }
}

#[cfg(feature = "futures-io")]
impl futures_util::AsyncRead for RecvStream {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        self.get_mut()
            .poll_read(cx, buf)
            .map_ok(Option::unwrap_or_default)
            .map_err(Into::into)
    }
}
