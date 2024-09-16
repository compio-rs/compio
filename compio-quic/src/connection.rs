use std::{
    collections::VecDeque,
    io,
    net::{IpAddr, SocketAddr},
    pin::{pin, Pin},
    sync::{Arc, Mutex, MutexGuard},
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

use compio_buf::{bytes::Bytes, BufResult};
use compio_log::{error, Instrument};
use compio_runtime::JoinHandle;
use flume::{Receiver, Sender};
use futures_util::{
    future::{self, Fuse, FusedFuture, LocalBoxFuture},
    select, stream, Future, FutureExt, StreamExt,
};
use quinn_proto::{
    congestion::Controller, crypto::rustls::HandshakeData, ConnectionHandle, ConnectionStats, Dir,
    EndpointEvent, StreamEvent, StreamId, VarInt,
};
use rustc_hash::FxHashMap as HashMap;
use thiserror::Error;

use crate::{RecvStream, SendStream, Socket};

#[derive(Debug)]
pub(crate) enum ConnectionEvent {
    Close(VarInt, Bytes),
    Proto(quinn_proto::ConnectionEvent),
}

#[derive(Debug)]
pub(crate) struct ConnectionState {
    pub(crate) conn: quinn_proto::Connection,
    pub(crate) error: Option<ConnectionError>,
    connected: bool,
    worker: Option<JoinHandle<()>>,
    poller: Option<Waker>,
    on_connected: Option<Waker>,
    on_handshake_data: Option<Waker>,
    datagram_received: VecDeque<Waker>,
    datagrams_unblocked: VecDeque<Waker>,
    stream_opened: [VecDeque<Waker>; 2],
    stream_available: [VecDeque<Waker>; 2],
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
        self.datagram_received.drain(..).for_each(Waker::wake);
        self.datagrams_unblocked.drain(..).for_each(Waker::wake);
        for e in &mut self.stream_opened {
            e.drain(..).for_each(Waker::wake);
        }
        for e in &mut self.stream_available {
            e.drain(..).for_each(Waker::wake);
        }
        wake_all_streams(&mut self.writable);
        wake_all_streams(&mut self.readable);
        wake_all_streams(&mut self.stopped);
    }

    fn close(&mut self, error_code: VarInt, reason: Bytes) {
        self.conn.close(Instant::now(), error_code, reason);
        self.terminate(ConnectionError::LocallyClosed);
        self.wake();
    }

    pub(crate) fn wake(&mut self) {
        if let Some(waker) = self.poller.take() {
            waker.wake()
        }
    }

    fn handshake_data(&self) -> Option<Box<HandshakeData>> {
        self.conn
            .crypto_session()
            .handshake_data()
            .map(|data| data.downcast::<HandshakeData>().unwrap())
    }

    pub(crate) fn check_0rtt(&self) -> bool {
        self.conn.side().is_server() || self.conn.is_handshaking() || self.conn.accepted_0rtt()
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
}

fn implicit_close(this: &Arc<ConnectionInner>) {
    if Arc::strong_count(this) == 2 {
        this.state().close(0u32.into(), Bytes::new())
    }
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
                poller: None,
                on_connected: None,
                on_handshake_data: None,
                datagram_received: VecDeque::new(),
                datagrams_unblocked: VecDeque::new(),
                stream_opened: [VecDeque::new(), VecDeque::new()],
                stream_available: [VecDeque::new(), VecDeque::new()],
                writable: HashMap::default(),
                readable: HashMap::default(),
                stopped: HashMap::default(),
            }),
            handle,
            socket,
            events_tx,
            events_rx,
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

    async fn run(self: &Arc<Self>) -> io::Result<()> {
        let mut poller = stream::poll_fn(|cx| {
            let mut state = self.state();
            let ready = state.poller.is_none();
            match &state.poller {
                Some(waker) if waker.will_wake(cx.waker()) => {}
                _ => state.poller = Some(cx.waker().clone()),
            };
            if ready {
                Poll::Ready(Some(()))
            } else {
                Poll::Pending
            }
        })
        .fuse();

        let mut timer = Timer::new();
        let mut event_stream = self.events_rx.stream().ready_chunks(100);
        let mut send_buf = Some(Vec::with_capacity(self.state().conn.current_mtu() as usize));
        let mut transmit_fut = pin!(Fuse::terminated());

        loop {
            let mut state = select! {
                _ = poller.select_next_some() => self.state(),
                _ = timer => {
                    timer.reset(None);
                    let mut state = self.state();
                    state.conn.handle_timeout(Instant::now());
                    state
                }
                events = event_stream.select_next_some() => {
                    let mut state = self.state();
                    for event in events {
                        match event {
                            ConnectionEvent::Close(error_code, reason) => state.close(error_code, reason),
                            ConnectionEvent::Proto(event) => state.conn.handle_event(event),
                        }
                    }
                    state
                },
                BufResult::<(), Vec<u8>>(res, mut buf) = transmit_fut => match res {
                    Ok(()) => {
                        buf.clear();
                        send_buf = Some(buf);
                        self.state()
                    },
                    Err(e) => break Err(e),
                },
            };

            if let Some(mut buf) = send_buf.take() {
                if let Some(transmit) = state.conn.poll_transmit(
                    Instant::now(),
                    self.socket.max_gso_segments(),
                    &mut buf,
                ) {
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
                        if state.conn.side().is_client() && !state.conn.accepted_0rtt() {
                            // Wake up rejected 0-RTT streams so they can fail immediately with
                            // `ZeroRttRejected` errors.
                            wake_all_streams(&mut state.writable);
                            wake_all_streams(&mut state.readable);
                            wake_all_streams(&mut state.stopped);
                        }
                    }
                    ConnectionLost { reason } => state.terminate(reason.into()),
                    Stream(StreamEvent::Readable { id }) => wake_stream(id, &mut state.readable),
                    Stream(StreamEvent::Writable { id }) => wake_stream(id, &mut state.writable),
                    Stream(StreamEvent::Finished { id }) => wake_stream(id, &mut state.stopped),
                    Stream(StreamEvent::Stopped { id, .. }) => {
                        wake_stream(id, &mut state.stopped);
                        wake_stream(id, &mut state.writable);
                    }
                    Stream(StreamEvent::Available { dir }) => state.stream_available[dir as usize]
                        .drain(..)
                        .for_each(Waker::wake),
                    Stream(StreamEvent::Opened { dir }) => state.stream_opened[dir as usize]
                        .drain(..)
                        .for_each(Waker::wake),
                    DatagramReceived => state.datagram_received.drain(..).for_each(Waker::wake),
                    DatagramsUnblocked => state.datagrams_unblocked.drain(..).for_each(Waker::wake),
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
            async move {
                #[allow(unused)]
                if let Err(e) = inner.run().await {
                    error!("I/O error: {}", e);
                }
            }
            .in_current_span()
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

    /// Convert into a 0-RTT or 0.5-RTT connection at the cost of weakened
    /// security.
    ///
    /// Returns `Ok` immediately if the local endpoint is able to attempt
    /// sending 0/0.5-RTT data. If so, the returned [`Connection`] can be used
    /// to send application data without waiting for the rest of the handshake
    /// to complete, at the cost of weakened cryptographic security guarantees.
    /// The [`Connection::accepted_0rtt`] method resolves when the handshake
    /// does complete, at which point subsequently opened streams and written
    /// data will have full cryptographic protection.
    ///
    /// ## Outgoing
    ///
    /// For outgoing connections, the initial attempt to convert to a
    /// [`Connection`] which sends 0-RTT data will proceed if the
    /// [`crypto::ClientConfig`][crate::crypto::ClientConfig] attempts to resume
    /// a previous TLS session. However, **the remote endpoint may not actually
    /// _accept_ the 0-RTT data**--yet still accept the connection attempt in
    /// general. This possibility is conveyed through the
    /// [`Connection::accepted_0rtt`] method--when the handshake completes, it
    /// resolves to true if the 0-RTT data was accepted and false if it was
    /// rejected. If it was rejected, the existence of streams opened and other
    /// application data sent prior to the handshake completing will not be
    /// conveyed to the remote application, and local operations on them will
    /// return `ZeroRttRejected` errors.
    ///
    /// A server may reject 0-RTT data at its discretion, but accepting 0-RTT
    /// data requires the relevant resumption state to be stored in the server,
    /// which servers may limit or lose for various reasons including not
    /// persisting resumption state across server restarts.
    ///
    /// ## Incoming
    ///
    /// For incoming connections, conversion to 0.5-RTT will always fully
    /// succeed. `into_0rtt` will always return `Ok` and
    /// [`Connection::accepted_0rtt`] will always resolve to true.
    ///
    /// ## Security
    ///
    /// On outgoing connections, this enables transmission of 0-RTT data, which
    /// is vulnerable to replay attacks, and should therefore never invoke
    /// non-idempotent operations.
    ///
    /// On incoming connections, this enables transmission of 0.5-RTT data,
    /// which may be sent before TLS client authentication has occurred, and
    /// should therefore not be used to send data for which client
    /// authentication is being used.
    pub fn into_0rtt(self) -> Result<Connection, Self> {
        let is_ok = {
            let state = self.0.state();
            state.conn.has_0rtt() || state.conn.side().is_server()
        };
        if is_ok {
            Ok(Connection(self.0.clone()))
        } else {
            Err(self)
        }
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
        implicit_close(&self.0)
    }
}

/// A QUIC connection.
#[derive(Debug, Clone)]
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
    /// [`ConnectionError::LocallyClosed`]. No more data is sent to the peer
    /// and the peer may drop buffered data upon receiving
    /// the CONNECTION_CLOSE frame.
    ///
    /// `error_code` and `reason` are not interpreted, and are provided directly
    /// to the peer.
    ///
    /// `reason` will be truncated to fit in a single packet with overhead; to
    /// improve odds that it is preserved in full, it should be kept under
    /// 1KiB.
    ///
    /// # Gracefully closing a connection
    ///
    /// Only the peer last receiving application data can be certain that all
    /// data is delivered. The only reliable action it can then take is to
    /// close the connection, potentially with a custom error code. The
    /// delivery of the final CONNECTION_CLOSE frame is very likely if both
    /// endpoints stay online long enough, and [`Endpoint::shutdown()`] can
    /// be used to provide sufficient time. Otherwise, the remote peer will
    /// time out the connection, provided that the idle timeout is not
    /// disabled.
    ///
    /// The sending side can not guarantee all stream data is delivered to the
    /// remote application. It only knows the data is delivered to the QUIC
    /// stack of the remote endpoint. Once the local side sends a
    /// CONNECTION_CLOSE frame in response to calling [`close()`] the remote
    /// endpoint may drop any data it received but is as yet undelivered to
    /// the application, including data that was acknowledged as received to
    /// the local endpoint.
    ///
    /// [`ConnectionError::LocallyClosed`]: ConnectionError::LocallyClosed
    /// [`Endpoint::shutdown()`]: crate::Endpoint::shutdown
    /// [`close()`]: Connection::close
    pub fn close(&self, error_code: VarInt, reason: &[u8]) {
        self.0
            .state()
            .close(error_code, Bytes::copy_from_slice(reason));
    }

    /// Wait for the connection to be closed for any reason.
    pub async fn closed(&self) -> ConnectionError {
        let worker = self.0.state().worker.take();
        if let Some(worker) = worker {
            let _ = worker.await;
        }

        self.0.try_state().unwrap_err()
    }

    /// If the connection is closed, the reason why.
    ///
    /// Returns `None` if the connection is still open.
    pub fn close_reason(&self) -> Option<ConnectionError> {
        self.0.try_state().err()
    }

    fn poll_recv_datagram(&self, cx: &mut Context) -> Poll<Result<Bytes, ConnectionError>> {
        let mut state = self.0.try_state()?;
        if let Some(bytes) = state.conn.datagrams().recv() {
            return Poll::Ready(Ok(bytes));
        }
        state.datagram_received.push_back(cx.waker().clone());
        Poll::Pending
    }

    /// Receive an application datagram.
    pub async fn recv_datagram(&self) -> Result<Bytes, ConnectionError> {
        future::poll_fn(|cx| self.poll_recv_datagram(cx)).await
    }

    fn try_send_datagram(
        &self,
        cx: Option<&mut Context>,
        data: Bytes,
    ) -> Result<(), Result<SendDatagramError, Bytes>> {
        use quinn_proto::SendDatagramError::*;
        let mut state = self.0.try_state().map_err(|e| Ok(e.into()))?;
        state
            .conn
            .datagrams()
            .send(data, cx.is_none())
            .map_err(|err| match err {
                UnsupportedByPeer => Ok(SendDatagramError::UnsupportedByPeer),
                Disabled => Ok(SendDatagramError::Disabled),
                TooLarge => Ok(SendDatagramError::TooLarge),
                Blocked(data) => {
                    state
                        .datagrams_unblocked
                        .push_back(cx.unwrap().waker().clone());
                    Err(data)
                }
            })?;
        state.wake();
        Ok(())
    }

    /// Transmit `data` as an unreliable, unordered application datagram.
    ///
    /// Application datagrams are a low-level primitive. They may be lost or
    /// delivered out of order, and `data` must both fit inside a single
    /// QUIC packet and be smaller than the maximum dictated by the peer.
    pub fn send_datagram(&self, data: Bytes) -> Result<(), SendDatagramError> {
        self.try_send_datagram(None, data).map_err(Result::unwrap)
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
        future::poll_fn(
            |cx| match self.try_send_datagram(Some(cx), data.take().unwrap()) {
                Ok(()) => Poll::Ready(Ok(())),
                Err(Ok(e)) => Poll::Ready(Err(e)),
                Err(Err(b)) => {
                    data.replace(b);
                    Poll::Pending
                }
            },
        )
        .await
    }

    fn poll_open_stream(
        &self,
        cx: Option<&mut Context>,
        dir: Dir,
    ) -> Poll<Result<(StreamId, bool), ConnectionError>> {
        let mut state = self.0.try_state()?;
        if let Some(stream) = state.conn.streams().open(dir) {
            Poll::Ready(Ok((
                stream,
                state.conn.side().is_client() && state.conn.is_handshaking(),
            )))
        } else {
            if let Some(cx) = cx {
                state.stream_available[dir as usize].push_back(cx.waker().clone());
            }
            Poll::Pending
        }
    }

    /// Initiate a new outgoing unidirectional stream.
    ///
    /// Streams are cheap and instantaneous to open. As a consequence, the peer
    /// won't be notified that a stream has been opened until the stream is
    /// actually used.
    pub fn open_uni(&self) -> Result<SendStream, OpenStreamError> {
        if let Poll::Ready((stream, is_0rtt)) = self.poll_open_stream(None, Dir::Uni)? {
            Ok(SendStream::new(self.0.clone(), stream, is_0rtt))
        } else {
            Err(OpenStreamError::StreamsExhausted)
        }
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
        let (stream, is_0rtt) =
            future::poll_fn(|cx| self.poll_open_stream(Some(cx), Dir::Uni)).await?;
        Ok(SendStream::new(self.0.clone(), stream, is_0rtt))
    }

    /// Initiate a new outgoing bidirectional stream.
    ///
    /// Streams are cheap and instantaneous to open. As a consequence, the peer
    /// won't be notified that a stream has been opened until the stream is
    /// actually used.
    pub fn open_bi(&self) -> Result<(SendStream, RecvStream), OpenStreamError> {
        if let Poll::Ready((stream, is_0rtt)) = self.poll_open_stream(None, Dir::Bi)? {
            Ok((
                SendStream::new(self.0.clone(), stream, is_0rtt),
                RecvStream::new(self.0.clone(), stream, is_0rtt),
            ))
        } else {
            Err(OpenStreamError::StreamsExhausted)
        }
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
        let (stream, is_0rtt) =
            future::poll_fn(|cx| self.poll_open_stream(Some(cx), Dir::Bi)).await?;
        Ok((
            SendStream::new(self.0.clone(), stream, is_0rtt),
            RecvStream::new(self.0.clone(), stream, is_0rtt),
        ))
    }

    fn poll_accept_stream(
        &self,
        cx: &mut Context,
        dir: Dir,
    ) -> Poll<Result<(StreamId, bool), ConnectionError>> {
        let mut state = self.0.try_state()?;
        if let Some(stream) = state.conn.streams().accept(dir) {
            state.wake();
            Poll::Ready(Ok((stream, state.conn.is_handshaking())))
        } else {
            state.stream_opened[dir as usize].push_back(cx.waker().clone());
            Poll::Pending
        }
    }

    /// Accept the next incoming uni-directional stream
    pub async fn accept_uni(&self) -> Result<RecvStream, ConnectionError> {
        let (stream, is_0rtt) = future::poll_fn(|cx| self.poll_accept_stream(cx, Dir::Uni)).await?;
        Ok(RecvStream::new(self.0.clone(), stream, is_0rtt))
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
        let (stream, is_0rtt) = future::poll_fn(|cx| self.poll_accept_stream(cx, Dir::Bi)).await?;
        Ok((
            SendStream::new(self.0.clone(), stream, is_0rtt),
            RecvStream::new(self.0.clone(), stream, is_0rtt),
        ))
    }

    /// Wait for the connection to be fully established.
    ///
    /// For clients, the resulting value indicates if 0-RTT was accepted. For
    /// servers, the resulting value is meaningless.
    pub async fn accepted_0rtt(&self) -> Result<bool, ConnectionError> {
        future::poll_fn(|cx| {
            let mut state = self.0.try_state()?;

            if state.connected {
                return Poll::Ready(Ok(state.conn.accepted_0rtt()));
            }

            match &state.on_connected {
                Some(waker) if waker.will_wake(cx.waker()) => {}
                _ => state.on_connected = Some(cx.waker().clone()),
            }

            Poll::Pending
        })
        .await
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
        implicit_close(&self.0)
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

/// Reasons why a connection might be lost
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ConnectionError {
    /// The peer doesn't implement any supported version
    #[error("peer doesn't implement any supported version")]
    VersionMismatch,
    /// The peer violated the QUIC specification as understood by this
    /// implementation
    #[error(transparent)]
    TransportError(#[from] quinn_proto::TransportError),
    /// The peer's QUIC stack aborted the connection automatically
    #[error("aborted by peer: {0}")]
    ConnectionClosed(quinn_proto::ConnectionClose),
    /// The peer closed the connection
    #[error("closed by peer: {0}")]
    ApplicationClosed(quinn_proto::ApplicationClose),
    /// The peer is unable to continue processing this connection, usually due
    /// to having restarted
    #[error("reset by peer")]
    Reset,
    /// Communication with the peer has lapsed for longer than the negotiated
    /// idle timeout
    ///
    /// If neither side is sending keep-alives, a connection will time out after
    /// a long enough idle period even if the peer is still reachable. See
    /// also [`TransportConfig::max_idle_timeout()`]
    /// and [`TransportConfig::keep_alive_interval()`].
    #[error("timed out")]
    TimedOut,
    /// The local application closed the connection
    #[error("closed")]
    LocallyClosed,
    /// The connection could not be created because not enough of the CID space
    /// is available
    ///
    /// Try using longer connection IDs.
    #[error("CIDs exhausted")]
    CidsExhausted,
}

impl From<quinn_proto::ConnectionError> for ConnectionError {
    fn from(value: quinn_proto::ConnectionError) -> Self {
        use quinn_proto::ConnectionError::*;

        match value {
            VersionMismatch => ConnectionError::VersionMismatch,
            TransportError(e) => ConnectionError::TransportError(e),
            ConnectionClosed(e) => ConnectionError::ConnectionClosed(e),
            ApplicationClosed(e) => ConnectionError::ApplicationClosed(e),
            Reset => ConnectionError::Reset,
            TimedOut => ConnectionError::TimedOut,
            LocallyClosed => ConnectionError::LocallyClosed,
            CidsExhausted => ConnectionError::CidsExhausted,
        }
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

#[cfg(feature = "h3")]
pub(crate) mod h3_impl {
    use compio_buf::bytes::{Buf, BytesMut};
    use futures_util::ready;
    use h3::{
        error::Code,
        ext::Datagram,
        quic::{self, Error, RecvDatagramExt, SendDatagramExt, WriteBuf},
    };

    use super::*;
    use crate::{send_stream::h3_impl::SendStream, ReadError, WriteError};

    impl Error for ConnectionError {
        fn is_timeout(&self) -> bool {
            matches!(self, ConnectionError::TimedOut)
        }

        fn err_code(&self) -> Option<u64> {
            match &self {
                ConnectionError::ApplicationClosed(quinn_proto::ApplicationClose {
                    error_code,
                    ..
                }) => Some(error_code.into_inner()),
                _ => None,
            }
        }
    }

    impl Error for SendDatagramError {
        fn is_timeout(&self) -> bool {
            false
        }

        fn err_code(&self) -> Option<u64> {
            match self {
                SendDatagramError::ConnectionLost(ConnectionError::ApplicationClosed(
                    quinn_proto::ApplicationClose { error_code, .. },
                )) => Some(error_code.into_inner()),
                _ => None,
            }
        }
    }

    impl<B> SendDatagramExt<B> for Connection
    where
        B: Buf,
    {
        type Error = SendDatagramError;

        fn send_datagram(&mut self, data: Datagram<B>) -> Result<(), Self::Error> {
            let mut buf = BytesMut::new();
            data.encode(&mut buf);
            Connection::send_datagram(self, buf.freeze())
        }
    }

    impl RecvDatagramExt for Connection {
        type Buf = Bytes;
        type Error = ConnectionError;

        fn poll_accept_datagram(
            &mut self,
            cx: &mut Context<'_>,
        ) -> Poll<Result<Option<Self::Buf>, Self::Error>> {
            Poll::Ready(Ok(Some(ready!(self.poll_recv_datagram(cx))?)))
        }
    }

    /// Bidirectional stream.
    pub struct BidiStream<B> {
        send: SendStream<B>,
        recv: RecvStream,
    }

    impl<B> BidiStream<B> {
        pub(crate) fn new(conn: Arc<ConnectionInner>, stream: StreamId, is_0rtt: bool) -> Self {
            Self {
                send: SendStream::new(conn.clone(), stream, is_0rtt),
                recv: RecvStream::new(conn, stream, is_0rtt),
            }
        }
    }

    impl<B> quic::BidiStream<B> for BidiStream<B>
    where
        B: Buf,
    {
        type RecvStream = RecvStream;
        type SendStream = SendStream<B>;

        fn split(self) -> (Self::SendStream, Self::RecvStream) {
            (self.send, self.recv)
        }
    }

    impl<B> quic::RecvStream for BidiStream<B>
    where
        B: Buf,
    {
        type Buf = Bytes;
        type Error = ReadError;

        fn poll_data(
            &mut self,
            cx: &mut Context<'_>,
        ) -> Poll<Result<Option<Self::Buf>, Self::Error>> {
            self.recv.poll_data(cx)
        }

        fn stop_sending(&mut self, error_code: u64) {
            self.recv.stop_sending(error_code)
        }

        fn recv_id(&self) -> quic::StreamId {
            self.recv.recv_id()
        }
    }

    impl<B> quic::SendStream<B> for BidiStream<B>
    where
        B: Buf,
    {
        type Error = WriteError;

        fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            self.send.poll_ready(cx)
        }

        fn send_data<T: Into<WriteBuf<B>>>(&mut self, data: T) -> Result<(), Self::Error> {
            self.send.send_data(data)
        }

        fn poll_finish(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            self.send.poll_finish(cx)
        }

        fn reset(&mut self, reset_code: u64) {
            self.send.reset(reset_code)
        }

        fn send_id(&self) -> quic::StreamId {
            self.send.send_id()
        }
    }

    impl<B> quic::SendStreamUnframed<B> for BidiStream<B>
    where
        B: Buf,
    {
        fn poll_send<D: Buf>(
            &mut self,
            cx: &mut Context<'_>,
            buf: &mut D,
        ) -> Poll<Result<usize, Self::Error>> {
            self.send.poll_send(cx, buf)
        }
    }

    /// Stream opener.
    #[derive(Clone)]
    pub struct OpenStreams(Connection);

    impl<B> quic::OpenStreams<B> for OpenStreams
    where
        B: Buf,
    {
        type BidiStream = BidiStream<B>;
        type OpenError = ConnectionError;
        type SendStream = SendStream<B>;

        fn poll_open_bidi(
            &mut self,
            cx: &mut Context<'_>,
        ) -> Poll<Result<Self::BidiStream, Self::OpenError>> {
            let (stream, is_0rtt) = ready!(self.0.poll_open_stream(Some(cx), Dir::Bi))?;
            Poll::Ready(Ok(BidiStream::new(self.0.0.clone(), stream, is_0rtt)))
        }

        fn poll_open_send(
            &mut self,
            cx: &mut Context<'_>,
        ) -> Poll<Result<Self::SendStream, Self::OpenError>> {
            let (stream, is_0rtt) = ready!(self.0.poll_open_stream(Some(cx), Dir::Uni))?;
            Poll::Ready(Ok(SendStream::new(self.0.0.clone(), stream, is_0rtt)))
        }

        fn close(&mut self, code: Code, reason: &[u8]) {
            self.0
                .close(code.value().try_into().expect("invalid code"), reason)
        }
    }

    impl<B> quic::OpenStreams<B> for Connection
    where
        B: Buf,
    {
        type BidiStream = BidiStream<B>;
        type OpenError = ConnectionError;
        type SendStream = SendStream<B>;

        fn poll_open_bidi(
            &mut self,
            cx: &mut Context<'_>,
        ) -> Poll<Result<Self::BidiStream, Self::OpenError>> {
            let (stream, is_0rtt) = ready!(self.poll_open_stream(Some(cx), Dir::Bi))?;
            Poll::Ready(Ok(BidiStream::new(self.0.clone(), stream, is_0rtt)))
        }

        fn poll_open_send(
            &mut self,
            cx: &mut Context<'_>,
        ) -> Poll<Result<Self::SendStream, Self::OpenError>> {
            let (stream, is_0rtt) = ready!(self.poll_open_stream(Some(cx), Dir::Uni))?;
            Poll::Ready(Ok(SendStream::new(self.0.clone(), stream, is_0rtt)))
        }

        fn close(&mut self, code: Code, reason: &[u8]) {
            Connection::close(self, code.value().try_into().expect("invalid code"), reason)
        }
    }

    impl<B> quic::Connection<B> for Connection
    where
        B: Buf,
    {
        type AcceptError = ConnectionError;
        type OpenStreams = OpenStreams;
        type RecvStream = RecvStream;

        fn poll_accept_recv(
            &mut self,
            cx: &mut std::task::Context<'_>,
        ) -> Poll<Result<Option<Self::RecvStream>, Self::AcceptError>> {
            let (stream, is_0rtt) = ready!(self.poll_accept_stream(cx, Dir::Uni))?;
            Poll::Ready(Ok(Some(RecvStream::new(self.0.clone(), stream, is_0rtt))))
        }

        fn poll_accept_bidi(
            &mut self,
            cx: &mut std::task::Context<'_>,
        ) -> Poll<Result<Option<Self::BidiStream>, Self::AcceptError>> {
            let (stream, is_0rtt) = ready!(self.poll_accept_stream(cx, Dir::Bi))?;
            Poll::Ready(Ok(Some(BidiStream::new(self.0.clone(), stream, is_0rtt))))
        }

        fn opener(&self) -> Self::OpenStreams {
            OpenStreams(self.clone())
        }
    }
}
