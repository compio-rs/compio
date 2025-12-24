use std::{
    io,
    mem::MaybeUninit,
    task::{Context, Poll},
};

use compio_buf::{BufResult, IoBufMut, bytes::Bytes};
use compio_io::AsyncRead;
use futures_util::future::poll_fn;
use quinn_proto::{Chunk, Chunks, ClosedStream, ReadableError, StreamId, VarInt};
use thiserror::Error;

use crate::{ConnectionError, ConnectionInner, StoppedError, sync::shared::Shared};

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
    conn: Shared<ConnectionInner>,
    stream: StreamId,
    is_0rtt: bool,
    all_data_read: bool,
    reset: Option<VarInt>,
}

impl RecvStream {
    pub(crate) fn new(conn: Shared<ConnectionInner>, stream: StreamId, is_0rtt: bool) -> Self {
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
        if self.is_0rtt && !state.check_0rtt() {
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

            if self.is_0rtt && !state.check_0rtt() {
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
        if self.is_0rtt && !state.check_0rtt() {
            return Poll::Ready(Err(ReadError::ZeroRttRejected));
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

    /// Attempts to read from the stream into the provided buffer
    ///
    /// On success, returns `Poll::Ready(Ok(num_bytes_read))` and places data
    /// into `buf`. If this returns zero bytes read (and `buf` has a
    /// non-zero length), that indicates that the remote
    /// side has [`finish`]ed the stream and the local side has already read all
    /// bytes.
    ///
    /// If no data is available for reading, this returns `Poll::Pending` and
    /// arranges for the current task (via `cx.waker()`) to be notified when
    /// the stream becomes readable or is closed.
    ///
    /// [`finish`]: crate::SendStream::finish
    pub fn poll_read_uninit(
        &mut self,
        cx: &mut Context,
        buf: &mut [MaybeUninit<u8>],
    ) -> Poll<Result<usize, ReadError>> {
        if buf.is_empty() {
            return Poll::Ready(Ok(0));
        }

        self.execute_poll_read(cx, true, |chunks| {
            let mut read = 0;
            loop {
                if read >= buf.len() {
                    // We know `read > 0` because `buf` cannot be empty here
                    return ReadStatus::Readable(read);
                }

                match chunks.next(buf.len() - read) {
                    Ok(Some(chunk)) => {
                        let bytes = chunk.bytes;
                        let len = bytes.len();
                        buf[read..read + len].copy_from_slice(unsafe {
                            std::slice::from_raw_parts(bytes.as_ptr().cast(), len)
                        });
                        read += len;
                    }
                    res => {
                        return (if read == 0 { None } else { Some(read) }, res.err()).into();
                    }
                }
            }
        })
        .map(|res| res.map(|n| n.unwrap_or_default()))
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
    /// If unordered reads have already been made, the resulting buffer may have
    /// gaps containing zeros.
    ///
    /// This operation is *not* cancel-safe.
    pub async fn read_to_end<B: IoBufMut>(&mut self, mut buf: B) -> BufResult<usize, B> {
        let mut start = u64::MAX;
        let mut end = 0;
        let mut chunks = vec![];
        loop {
            let chunk = match self.read_chunk(usize::MAX, false).await {
                Ok(Some(chunk)) => chunk,
                Ok(None) => break,
                Err(e) => return BufResult(Err(e.into()), buf),
            };
            start = start.min(chunk.offset);
            end = end.max(chunk.offset + chunk.bytes.len() as u64);
            chunks.push((chunk.offset, chunk.bytes));
        }
        if start == u64::MAX || start >= end {
            // no data read
            return BufResult(Ok(0), buf);
        }
        let len = (end - start) as usize;
        let cap = buf.buf_capacity();
        let needed = len.saturating_sub(cap);
        if needed > 0
            && let Err(e) = buf.reserve(needed)
        {
            return BufResult(Err(io::Error::new(io::ErrorKind::InvalidData, e)), buf);
        }
        let slice = &mut buf.as_uninit()[..len];
        slice.fill(MaybeUninit::new(0));
        for (offset, bytes) in chunks {
            let offset = (offset - start) as usize;
            let buf_len = bytes.len();
            slice[offset..offset + buf_len].copy_from_slice(unsafe {
                std::slice::from_raw_parts(bytes.as_ptr().cast(), buf_len)
            });
        }
        unsafe { buf.advance_to(len) }
        BufResult(Ok(len), buf)
    }
}

impl Drop for RecvStream {
    fn drop(&mut self) {
        let mut state = self.conn.state();

        // clean up any previously registered wakers
        state.readable.remove(&self.stream);

        if state.error.is_some() || (self.is_0rtt && !state.check_0rtt()) {
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
            IllegalOrderedRead => io::ErrorKind::InvalidInput,
        };
        Self::new(kind, x)
    }
}

/// Errors that arise from reading from a stream.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ReadExactError {
    /// The stream finished before all bytes were read
    #[error("stream finished early (expected {0} bytes more)")]
    FinishedEarly(usize),
    /// A read error occurred
    #[error(transparent)]
    ReadError(#[from] ReadError),
}

impl AsyncRead for RecvStream {
    async fn read<B: IoBufMut>(&mut self, mut buf: B) -> BufResult<usize, B> {
        let res = poll_fn(|cx| self.poll_read_uninit(cx, buf.as_uninit()))
            .await
            .inspect(|&n| unsafe { buf.advance_to(n) })
            .map_err(Into::into);
        BufResult(res, buf)
    }
}

#[cfg(feature = "io-compat")]
impl futures_util::AsyncRead for RecvStream {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        // SAFETY: buf is valid
        self.get_mut()
            .poll_read_uninit(cx, unsafe {
                std::slice::from_raw_parts_mut(buf.as_mut_ptr().cast(), buf.len())
            })
            .map_err(Into::into)
    }
}

#[cfg(feature = "h3")]
pub(crate) mod h3_impl {
    use h3::quic::{self, StreamErrorIncoming};

    use super::*;

    impl From<ReadError> for StreamErrorIncoming {
        fn from(e: ReadError) -> Self {
            use ReadError::*;
            match e {
                Reset(code) => Self::StreamTerminated {
                    error_code: code.into_inner(),
                },
                ConnectionLost(e) => Self::ConnectionErrorIncoming {
                    connection_error: e.into(),
                },
                IllegalOrderedRead => unreachable!("illegal ordered read"),
                e => Self::Unknown(Box::new(e)),
            }
        }
    }

    impl quic::RecvStream for RecvStream {
        type Buf = Bytes;

        fn poll_data(
            &mut self,
            cx: &mut Context<'_>,
        ) -> Poll<Result<Option<Self::Buf>, StreamErrorIncoming>> {
            self.execute_poll_read(cx, true, |chunks| match chunks.next(usize::MAX) {
                Ok(Some(chunk)) => ReadStatus::Readable(chunk.bytes),
                res => (None, res.err()).into(),
            })
            .map_err(Into::into)
        }

        fn stop_sending(&mut self, error_code: u64) {
            self.stop(error_code.try_into().expect("invalid error_code"))
                .ok();
        }

        fn recv_id(&self) -> quic::StreamId {
            u64::from(self.stream).try_into().unwrap()
        }
    }
}
