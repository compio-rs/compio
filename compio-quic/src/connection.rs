use std::{
    collections::HashMap,
    io,
    net::{IpAddr, SocketAddr},
    pin::{pin, Pin},
    sync::{Arc, Mutex, MutexGuard},
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

use bytes::Bytes;
use compio_buf::BufResult;
use compio_runtime::JoinHandle;
use event_listener::{Event, IntoNotification};
use flume::{Receiver, Sender};
use futures_util::{
    future::{self, Fuse, FusedFuture, LocalBoxFuture},
    select, stream, Future, FutureExt, StreamExt,
};
use quinn_proto::{
    congestion::Controller, crypto::rustls::HandshakeData, ConnectionError, ConnectionHandle,
    ConnectionStats, Dir, EndpointEvent, StreamEvent, StreamId, VarInt,
};
use thiserror::Error;

use crate::{wait_event, RecvStream, SendStream, Socket};

#[derive(Debug)]
pub(crate) enum ConnectionEvent {
    Close(VarInt, String),
    Proto(quinn_proto::ConnectionEvent),
}

#[derive(Debug)]
pub(crate) struct ConnectionState {
    pub(crate) conn: quinn_proto::Connection,
    pub(crate) error: Option<ConnectionError>,
    connected: bool,
    worker: Option<JoinHandle<()>>,
    poll_waker: Option<Waker>,
    on_connected: Option<Waker>,
    on_handshake_data: Option<Waker>,
    pub(crate) writable: HashMap<StreamId, Waker>,
    pub(crate) readable: HashMap<StreamId, Waker>,
    pub(crate) stopped: HashMap<StreamId, Waker>,
}

impl ConnectionState {
    fn terminate(&mut self, reason: ConnectionError) {
        self.error = Some(reason);
        self.connected = false;

        if let Some(waker) = self.on_handshake_data.take() {
            waker.wake()
        }
        if let Some(waker) = self.on_connected.take() {
            waker.wake()
        }
        wake_all_streams(&mut self.writable);
        wake_all_streams(&mut self.readable);
        wake_all_streams(&mut self.stopped);
    }

    pub(crate) fn wake(&mut self) {
        if let Some(waker) = self.poll_waker.take() {
            waker.wake()
        }
    }

    fn handshake_data(&self) -> Option<Box<HandshakeData>> {
        self.conn
            .crypto_session()
            .handshake_data()
            .map(|data| data.downcast::<HandshakeData>().unwrap())
    }
}

fn wake_stream(stream: StreamId, wakers: &mut HashMap<StreamId, Waker>) {
    if let Some(waker) = wakers.remove(&stream) {
        waker.wake();
    }
}

fn wake_all_streams(wakers: &mut HashMap<StreamId, Waker>) {
    wakers.drain().for_each(|(_, waker)| waker.wake())
}

#[derive(Debug)]
pub(crate) struct ConnectionInner {
    state: Mutex<ConnectionState>,
    handle: ConnectionHandle,
    socket: Socket,
    events_tx: Sender<(ConnectionHandle, EndpointEvent)>,
    events_rx: Receiver<ConnectionEvent>,
    datagram_received: Event,
    datagrams_unblocked: Event,
    stream_opened: [Event; 2],
    stream_available: [Event; 2],
}

impl ConnectionInner {
    fn new(
        handle: ConnectionHandle,
        conn: quinn_proto::Connection,
        socket: Socket,
        events_tx: Sender<(ConnectionHandle, EndpointEvent)>,
        events_rx: Receiver<ConnectionEvent>,
    ) -> Self {
        Self {
            state: Mutex::new(ConnectionState {
                conn,
                connected: false,
                error: None,
                worker: None,
                poll_waker: None,
                on_connected: None,
                on_handshake_data: None,
                writable: HashMap::new(),
                readable: HashMap::new(),
                stopped: HashMap::new(),
            }),
            handle,
            socket,
            events_tx,
            events_rx,
            datagram_received: Event::new(),
            datagrams_unblocked: Event::new(),
            stream_opened: [Event::new(), Event::new()],
            stream_available: [Event::new(), Event::new()],
        }
    }

    #[inline]
    pub(crate) fn state(&self) -> MutexGuard<ConnectionState> {
        self.state.lock().unwrap()
    }

    #[inline]
    pub(crate) fn try_state(&self) -> Result<MutexGuard<ConnectionState>, ConnectionError> {
        let state = self.state();
        if let Some(error) = &state.error {
            Err(error.clone())
        } else {
            Ok(state)
        }
    }

    fn notify_events(&self) {
        self.datagram_received.notify(usize::MAX.additional());
        self.datagrams_unblocked.notify(usize::MAX.additional());
        for e in &self.stream_opened {
            e.notify(usize::MAX.additional());
        }
        for e in &self.stream_available {
            e.notify(usize::MAX.additional());
        }
    }

    fn close(&self, error_code: VarInt, reason: String) {
        let mut state = self.state();
        state.conn.close(Instant::now(), error_code, reason.into());
        state.terminate(ConnectionError::LocallyClosed);
        state.wake();
        self.notify_events();
    }

    async fn run(&self) -> io::Result<()> {
        let mut send_buf = Some(Vec::with_capacity(self.state().conn.current_mtu() as usize));
        let mut transmit_fut = pin!(Fuse::terminated());

        let mut timer = Timer::new();

        let mut poller = stream::poll_fn(|cx| {
            let mut state = self.state();
            let ready = state.poll_waker.is_none();
            match &state.poll_waker {
                Some(waker) if waker.will_wake(cx.waker()) => {}
                _ => state.poll_waker = Some(cx.waker().clone()),
            };
            if ready {
                Poll::Ready(Some(()))
            } else {
                Poll::Pending
            }
        })
        .fuse();

        loop {
            select! {
                _ = poller.next() => {}
                _ = timer => {
                    self.state().conn.handle_timeout(Instant::now());
                    timer.reset(None);
                }
                ev = self.events_rx.recv_async() => match ev {
                    Ok(ConnectionEvent::Close(error_code, reason)) => self.close(error_code, reason),
                    Ok(ConnectionEvent::Proto(ev)) => self.state().conn.handle_event(ev),
                    Err(_) => unreachable!("endpoint dropped connection"),
                },
                BufResult::<(), Vec<u8>>(res, mut buf) = transmit_fut => match res {
                    Ok(()) => {
                        buf.clear();
                        send_buf = Some(buf);
                    },
                    Err(e) => break Err(e),
                },
            }

            let now = Instant::now();
            let mut state = self.state();

            if let Some(mut buf) = send_buf.take() {
                if let Some(transmit) =
                    state
                        .conn
                        .poll_transmit(now, self.socket.max_gso_segments(), &mut buf)
                {
                    transmit_fut.set(async move { self.socket.send(buf, &transmit).await }.fuse())
                } else {
                    send_buf = Some(buf);
                }
            }

            timer.reset(state.conn.poll_timeout());

            while let Some(event) = state.conn.poll_endpoint_events() {
                let _ = self.events_tx.send((self.handle, event));
            }

            while let Some(event) = state.conn.poll() {
                use quinn_proto::Event::*;
                match event {
                    HandshakeDataReady => {
                        if let Some(waker) = state.on_handshake_data.take() {
                            waker.wake()
                        }
                    }
                    Connected => {
                        state.connected = true;
                        if let Some(waker) = state.on_connected.take() {
                            waker.wake()
                        }
                    }
                    ConnectionLost { reason } => {
                        state.terminate(reason);
                        self.notify_events();
                    }
                    Stream(StreamEvent::Readable { id }) => wake_stream(id, &mut state.readable),
                    Stream(StreamEvent::Writable { id }) => wake_stream(id, &mut state.writable),
                    Stream(StreamEvent::Finished { id }) => wake_stream(id, &mut state.stopped),
                    Stream(StreamEvent::Stopped { id, .. }) => {
                        wake_stream(id, &mut state.stopped);
                        wake_stream(id, &mut state.writable);
                    }
                    Stream(StreamEvent::Available { dir }) => {
                        self.stream_available[dir as usize].notify(usize::MAX.additional());
                    }
                    Stream(StreamEvent::Opened { dir }) => {
                        self.stream_opened[dir as usize].notify(usize::MAX.additional());
                    }
                    DatagramReceived => {
                        self.datagram_received.notify(usize::MAX.additional());
                    }
                    DatagramsUnblocked => {
                        self.datagrams_unblocked.notify(usize::MAX.additional());
                    }
                }
            }

            if state.conn.is_drained() {
                break Ok(());
            }
        }
    }
}

macro_rules! conn_fn {
    () => {
        /// The local IP address which was used when the peer established
        /// the connection.
        ///
        /// This can be different from the address the endpoint is bound to, in case
        /// the endpoint is bound to a wildcard address like `0.0.0.0` or `::`.
        ///
        /// This will return `None` for clients, or when the platform does not
        /// expose this information.
        pub fn local_ip(&self) -> Option<IpAddr> {
            self.0.state().conn.local_ip()
        }

        /// The peer's UDP address.
        ///
        /// Will panic if called after `poll` has returned `Ready`.
        pub fn remote_address(&self) -> SocketAddr {
            self.0.state().conn.remote_address()
        }

        /// Current best estimate of this connection's latency (round-trip-time).
        pub fn rtt(&self) -> Duration {
            self.0.state().conn.rtt()
        }

        /// Connection statistics.
        pub fn stats(&self) -> ConnectionStats {
            self.0.state().conn.stats()
        }

        /// Current state of the congestion control algorithm. (For debugging
        /// purposes)
        pub fn congestion_state(&self) -> Box<dyn Controller> {
            self.0.state().conn.congestion_state().clone_box()
        }

        /// Cryptographic identity of the peer.
        pub fn peer_identity(
            &self,
        ) -> Option<Box<Vec<rustls::pki_types::CertificateDer<'static>>>> {
            self.0
                .state()
                .conn
                .crypto_session()
                .peer_identity()
                .map(|v| v.downcast().unwrap())
        }

        /// Derive keying material from this connection's TLS session secrets.
        ///
        /// When both peers call this method with the same `label` and `context`
        /// arguments and `output` buffers of equal length, they will get the
        /// same sequence of bytes in `output`. These bytes are cryptographically
        /// strong and pseudorandom, and are suitable for use as keying material.
        ///
        /// This function fails if called with an empty `output` or called prior to
        /// the handshake completing.
        ///
        /// See [RFC5705](https://tools.ietf.org/html/rfc5705) for more information.
        pub fn export_keying_material(
            &self,
            output: &mut [u8],
            label: &[u8],
            context: &[u8],
        ) -> Result<(), quinn_proto::crypto::ExportKeyingMaterialError> {
            self.0
                .state()
                .conn
                .crypto_session()
                .export_keying_material(output, label, context)
        }
    };
}

/// In-progress connection attempt future
#[derive(Debug)]
#[must_use = "futures/streams/sinks do nothing unless you `.await` or poll them"]
pub struct Connecting(Arc<ConnectionInner>);

impl Connecting {
    conn_fn!();

    pub(crate) fn new(
        handle: ConnectionHandle,
        conn: quinn_proto::Connection,
        socket: Socket,
        events_tx: Sender<(ConnectionHandle, EndpointEvent)>,
        events_rx: Receiver<ConnectionEvent>,
    ) -> Self {
        let inner = Arc::new(ConnectionInner::new(
            handle, conn, socket, events_tx, events_rx,
        ));
        let worker = compio_runtime::spawn({
            let inner = inner.clone();
            async move { inner.run().await.unwrap() }
        });
        inner.state().worker = Some(worker);
        Self(inner)
    }

    /// Parameters negotiated during the handshake.
    pub async fn handshake_data(&mut self) -> Result<Box<HandshakeData>, ConnectionError> {
        future::poll_fn(|cx| {
            let mut state = self.0.try_state()?;
            if let Some(data) = state.handshake_data() {
                return Poll::Ready(Ok(data));
            }

            match &state.on_handshake_data {
                Some(waker) if waker.will_wake(cx.waker()) => {}
                _ => state.on_handshake_data = Some(cx.waker().clone()),
            }

            Poll::Pending
        })
        .await
    }
}

impl Future for Connecting {
    type Output = Result<Connection, ConnectionError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.0.try_state()?;

        if state.connected {
            return Poll::Ready(Ok(Connection(self.0.clone())));
        }

        match &state.on_connected {
            Some(waker) if waker.will_wake(cx.waker()) => {}
            _ => state.on_connected = Some(cx.waker().clone()),
        }

        Poll::Pending
    }
}

impl Drop for Connecting {
    fn drop(&mut self) {
        if Arc::strong_count(&self.0) == 2 {
            self.0.close(0u32.into(), String::new())
        }
    }
}

/// A QUIC connection.
#[derive(Debug)]
pub struct Connection(Arc<ConnectionInner>);

impl Connection {
    conn_fn!();

    /// Parameters negotiated during the handshake.
    pub fn handshake_data(&mut self) -> Result<Box<HandshakeData>, ConnectionError> {
        Ok(self.0.try_state()?.handshake_data().unwrap())
    }

    /// Compute the maximum size of datagrams that may be passed to
    /// [`send_datagram()`](Self::send_datagram).
    ///
    /// Returns `None` if datagrams are unsupported by the peer or disabled
    /// locally.
    ///
    /// This may change over the lifetime of a connection according to variation
    /// in the path MTU estimate. The peer can also enforce an arbitrarily small
    /// fixed limit, but if the peer's limit is large this is guaranteed to be a
    /// little over a kilobyte at minimum.
    ///
    /// Not necessarily the maximum size of received datagrams.
    pub fn max_datagram_size(&self) -> Option<usize> {
        self.0.state().conn.datagrams().max_size()
    }

    /// Bytes available in the outgoing datagram buffer.
    ///
    /// When greater than zero, calling [`send_datagram()`](Self::send_datagram)
    /// with a datagram of at most this size is guaranteed not to cause older
    /// datagrams to be dropped.
    pub fn datagram_send_buffer_space(&self) -> usize {
        self.0.state().conn.datagrams().send_buffer_space()
    }

    /// Modify the number of remotely initiated unidirectional streams that may
    /// be concurrently open.
    ///
    /// No streams may be opened by the peer unless fewer than `count` are
    /// already open. Large `count`s increase both minimum and worst-case
    /// memory consumption.
    pub fn set_max_concurrent_uni_streams(&self, count: VarInt) {
        let mut state = self.0.state();
        state.conn.set_max_concurrent_streams(Dir::Uni, count);
        // May need to send MAX_STREAMS to make progress
        state.wake();
    }

    /// See [`quinn_proto::TransportConfig::receive_window()`]
    pub fn set_receive_window(&self, receive_window: VarInt) {
        let mut state = self.0.state();
        state.conn.set_receive_window(receive_window);
        state.wake();
    }

    /// Modify the number of remotely initiated bidirectional streams that may
    /// be concurrently open.
    ///
    /// No streams may be opened by the peer unless fewer than `count` are
    /// already open. Large `count`s increase both minimum and worst-case
    /// memory consumption.
    pub fn set_max_concurrent_bi_streams(&self, count: VarInt) {
        let mut state = self.0.state();
        state.conn.set_max_concurrent_streams(Dir::Bi, count);
        // May need to send MAX_STREAMS to make progress
        state.wake();
    }

    /// Close the connection immediately.
    ///
    /// Pending operations will fail immediately with
    /// [`ConnectionError::LocallyClosed`]. Delivery of data on unfinished
    /// streams is not guaranteed, so the application must call this only when
    /// all important communications have been completed, e.g. by calling
    /// [`finish`] on outstanding [`SendStream`]s and waiting for the resulting
    /// futures to complete.
    ///
    /// `error_code` and `reason` are not interpreted, and are provided directly
    /// to the peer.
    ///
    /// `reason` will be truncated to fit in a single packet with overhead; to
    /// improve odds that it is preserved in full, it should be kept under 1KiB.
    ///
    /// [`ConnectionError::LocallyClosed`]: quinn_proto::ConnectionError::LocallyClosed
    /// [`finish`]: crate::SendStream::finish
    /// [`SendStream`]: crate::SendStream
    pub fn close(&self, error_code: VarInt, reason: &str) {
        self.0.close(error_code, reason.to_string());
    }

    /// Wait for the connection to be closed for any reason.
    pub async fn closed(&self) -> ConnectionError {
        let worker = self.0.state().worker.take();
        if let Some(worker) = worker {
            let _ = worker.await;
        }

        self.0.state().error.clone().unwrap()
    }

    /// Receive an application datagram.
    pub async fn recv_datagram(&self) -> Result<Bytes, ConnectionError> {
        let bytes = wait_event!(
            self.0.datagram_received,
            if let Some(bytes) = self.0.try_state()?.conn.datagrams().recv() {
                break bytes;
            }
        );
        Ok(bytes)
    }

    fn try_send_datagram(
        &self,
        data: Bytes,
        drop: bool,
    ) -> Result<(), Result<SendDatagramError, Bytes>> {
        let mut state = self.0.try_state().map_err(|e| Ok(e.into()))?;
        state
            .conn
            .datagrams()
            .send(data, drop)
            .map_err(TryInto::try_into)?;
        state.wake();
        Ok(())
    }

    /// Transmit `data` as an unreliable, unordered application datagram.
    ///
    /// Application datagrams are a low-level primitive. They may be lost or
    /// delivered out of order, and `data` must both fit inside a single
    /// QUIC packet and be smaller than the maximum dictated by the peer.
    pub fn send_datagram(&self, data: Bytes) -> Result<(), SendDatagramError> {
        self.try_send_datagram(data, true).map_err(Result::unwrap)
    }

    /// Transmit `data` as an unreliable, unordered application datagram.
    ///
    /// Unlike [`send_datagram()`], this method will wait for buffer space
    /// during congestion conditions, which effectively prioritizes old
    /// datagrams over new datagrams.
    ///
    /// See [`send_datagram()`] for details.
    ///
    /// [`send_datagram()`]: Connection::send_datagram
    pub async fn send_datagram_wait(&self, data: Bytes) -> Result<(), SendDatagramError> {
        let mut data = Some(data);
        wait_event!(
            self.0.datagrams_unblocked,
            match self.try_send_datagram(data.take().unwrap(), false) {
                Ok(res) => break Ok(res),
                Err(Ok(e)) => break Err(e),
                Err(Err(b)) => data.replace(b),
            }
        )
    }

    fn try_open_stream(&self, dir: Dir) -> Result<StreamId, OpenStreamError> {
        self.0
            .try_state()?
            .conn
            .streams()
            .open(dir)
            .ok_or(OpenStreamError::StreamsExhausted)
    }

    async fn open_stream(&self, dir: Dir) -> Result<StreamId, ConnectionError> {
        wait_event!(
            self.0.stream_available[dir as usize],
            match self.try_open_stream(dir) {
                Ok(stream) => break Ok(stream),
                Err(OpenStreamError::StreamsExhausted) => {}
                Err(OpenStreamError::ConnectionLost(e)) => break Err(e),
            }
        )
    }

    /// Initiate a new outgoing unidirectional stream.
    ///
    /// Streams are cheap and instantaneous to open. As a consequence, the peer
    /// won't be notified that a stream has been opened until the stream is
    /// actually used.
    pub fn open_uni(&self) -> Result<SendStream, OpenStreamError> {
        let stream = self.try_open_stream(Dir::Uni)?;
        Ok(SendStream::new(self.0.clone(), stream))
    }

    /// Initiate a new outgoing unidirectional stream.
    ///
    /// Unlike [`open_uni()`], this method will wait for the connection to allow
    /// a new stream to be opened.
    ///
    /// See [`open_uni()`] for details.
    ///
    /// [`open_uni()`]: crate::Connection::open_uni
    pub async fn open_uni_wait(&self) -> Result<SendStream, ConnectionError> {
        let stream = self.open_stream(Dir::Uni).await?;
        Ok(SendStream::new(self.0.clone(), stream))
    }

    /// Initiate a new outgoing bidirectional stream.
    ///
    /// Streams are cheap and instantaneous to open. As a consequence, the peer
    /// won't be notified that a stream has been opened until the stream is
    /// actually used.
    pub fn open_bi(&self) -> Result<(SendStream, RecvStream), OpenStreamError> {
        let stream = self.try_open_stream(Dir::Bi)?;
        Ok((
            SendStream::new(self.0.clone(), stream),
            RecvStream::new(self.0.clone(), stream),
        ))
    }

    /// Initiate a new outgoing bidirectional stream.
    ///
    /// Unlike [`open_bi()`], this method will wait for the connection to allow
    /// a new stream to be opened.
    ///
    /// See [`open_bi()`] for details.
    ///
    /// [`open_bi()`]: crate::Connection::open_bi
    pub async fn open_bi_wait(&self) -> Result<(SendStream, RecvStream), ConnectionError> {
        let stream = self.open_stream(Dir::Bi).await?;
        Ok((
            SendStream::new(self.0.clone(), stream),
            RecvStream::new(self.0.clone(), stream),
        ))
    }

    async fn accept_stream(&self, dir: Dir) -> Result<StreamId, ConnectionError> {
        wait_event!(self.0.stream_opened[dir as usize], {
            let mut state = self.0.state();
            if let Some(stream) = state.conn.streams().accept(dir) {
                state.wake();
                break Ok(stream);
            } else if let Some(error) = &state.error {
                break Err(error.clone());
            }
        })
    }

    /// Accept the next incoming uni-directional stream
    pub async fn accept_uni(&self) -> Result<RecvStream, ConnectionError> {
        let stream = self.accept_stream(Dir::Uni).await?;
        Ok(RecvStream::new(self.0.clone(), stream))
    }

    /// Accept the next incoming bidirectional stream
    ///
    /// **Important Note**: The `Connection` that calls [`open_bi()`] must write
    /// to its [`SendStream`] before the other `Connection` is able to
    /// `accept_bi()`. Calling [`open_bi()`] then waiting on the [`RecvStream`]
    /// without writing anything to [`SendStream`] will never succeed.
    ///
    /// [`accept_bi()`]: crate::Connection::accept_bi
    /// [`open_bi()`]: crate::Connection::open_bi
    /// [`SendStream`]: crate::SendStream
    /// [`RecvStream`]: crate::RecvStream
    pub async fn accept_bi(&self) -> Result<(SendStream, RecvStream), ConnectionError> {
        let stream = self.accept_stream(Dir::Bi).await?;
        Ok((
            SendStream::new(self.0.clone(), stream),
            RecvStream::new(self.0.clone(), stream),
        ))
    }
}

impl PartialEq for Connection {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Connection {}

impl Drop for Connection {
    fn drop(&mut self) {
        if Arc::strong_count(&self.0) == 2 {
            self.close(0u32.into(), "")
        }
    }
}

struct Timer {
    deadline: Option<Instant>,
    fut: Fuse<LocalBoxFuture<'static, ()>>,
}

impl Timer {
    fn new() -> Self {
        Self {
            deadline: None,
            fut: Fuse::terminated(),
        }
    }

    fn reset(&mut self, deadline: Option<Instant>) {
        if let Some(deadline) = deadline {
            if self.deadline.is_none() || self.deadline != Some(deadline) {
                self.fut = compio_runtime::time::sleep_until(deadline)
                    .boxed_local()
                    .fuse();
            }
        } else {
            self.fut = Fuse::terminated();
        }
        self.deadline = deadline;
    }
}

impl Future for Timer {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.fut.poll_unpin(cx)
    }
}

impl FusedFuture for Timer {
    fn is_terminated(&self) -> bool {
        self.fut.is_terminated()
    }
}

/// Errors that can arise when sending a datagram
#[derive(Debug, Error, Clone, Eq, PartialEq)]
pub enum SendDatagramError {
    /// The peer does not support receiving datagram frames
    #[error("datagrams not supported by peer")]
    UnsupportedByPeer,
    /// Datagram support is disabled locally
    #[error("datagram support disabled")]
    Disabled,
    /// The datagram is larger than the connection can currently accommodate
    ///
    /// Indicates that the path MTU minus overhead or the limit advertised by
    /// the peer has been exceeded.
    #[error("datagram too large")]
    TooLarge,
    /// The connection was lost
    #[error("connection lost")]
    ConnectionLost(#[from] ConnectionError),
}

impl TryFrom<quinn_proto::SendDatagramError> for SendDatagramError {
    type Error = Bytes;

    fn try_from(value: quinn_proto::SendDatagramError) -> Result<Self, Self::Error> {
        use quinn_proto::SendDatagramError::*;
        match value {
            UnsupportedByPeer => Ok(SendDatagramError::UnsupportedByPeer),
            Disabled => Ok(SendDatagramError::Disabled),
            TooLarge => Ok(SendDatagramError::TooLarge),
            Blocked(data) => Err(data),
        }
    }
}

/// Errors that can arise when trying to open a stream
#[derive(Debug, Error, Clone, Eq, PartialEq)]
pub enum OpenStreamError {
    /// The connection was lost
    #[error("connection lost")]
    ConnectionLost(#[from] ConnectionError),
    // The streams in the given direction are currently exhausted
    #[error("streams exhausted")]
    StreamsExhausted,
}
