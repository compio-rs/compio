use std::{
    io,
    net::{IpAddr, SocketAddr},
    pin::{pin, Pin},
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

use compio_buf::BufResult;
use compio_runtime::JoinHandle;
use flume::{Receiver, Sender};
use futures_util::{
    future::{self, Fuse, FusedFuture, LocalBoxFuture},
    select, stream, Future, FutureExt, StreamExt,
};
use quinn_proto::{
    congestion::Controller, crypto::rustls::HandshakeData, ConnectionError, ConnectionHandle,
    ConnectionStats, Dir, EndpointEvent, VarInt,
};

use crate::Socket;

#[derive(Debug)]
pub(crate) enum ConnectionEvent {
    Close(VarInt, String),
    Proto(quinn_proto::ConnectionEvent),
}

#[derive(Debug)]
struct ConnectionState {
    conn: quinn_proto::Connection,
    connected: bool,
    error: Option<ConnectionError>,
    worker: Option<JoinHandle<()>>,
    poll_waker: Option<Waker>,
    on_connected: Option<Waker>,
    on_handshake_data: Option<Waker>,
}

impl ConnectionState {
    fn terminate(&mut self, reason: ConnectionError) {
        self.error = Some(reason);
        self.connected = false;

        if let Some(waker) = self.on_connected.take() {
            waker.wake()
        }
        if let Some(waker) = self.on_handshake_data.take() {
            waker.wake()
        }
    }

    fn wake(&mut self) {
        if let Some(waker) = self.poll_waker.take() {
            waker.wake()
        }
    }

    #[inline]
    fn try_map<T>(&self, f: impl Fn(&Self) -> Option<T>) -> Option<Result<T, ConnectionError>> {
        if let Some(error) = &self.error {
            Some(Err(error.clone()))
        } else {
            f(self).map(Ok)
        }
    }

    #[inline]
    fn try_handshake_data(&self) -> Option<Result<Box<HandshakeData>, ConnectionError>> {
        self.try_map(|state| {
            state
                .conn
                .crypto_session()
                .handshake_data()
                .map(|data| data.downcast::<HandshakeData>().unwrap())
        })
    }
}

#[derive(Debug)]
struct ConnectionInner {
    state: Mutex<ConnectionState>,
    handle: ConnectionHandle,
    socket: Socket,
    events_tx: Sender<(ConnectionHandle, EndpointEvent)>,
    events_rx: Receiver<ConnectionEvent>,
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
            }),
            handle,
            socket,
            events_tx,
            events_rx,
        }
    }

    fn close(&self, error_code: VarInt, reason: String) {
        let mut state = self.state.lock().unwrap();
        state.conn.close(Instant::now(), error_code, reason.into());
        state.terminate(ConnectionError::LocallyClosed);
        state.wake();
    }

    async fn run(&self) -> io::Result<()> {
        let mut send_buf = Some(Vec::with_capacity(
            self.state.lock().unwrap().conn.current_mtu() as usize,
        ));
        let mut transmit_fut = pin!(Fuse::terminated());

        let mut timer = Timer::new();

        let mut poller = stream::poll_fn(|cx| {
            let mut state = self.state.lock().unwrap();
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
                    self.state.lock().unwrap().conn.handle_timeout(Instant::now());
                    timer.reset(None);
                }
                ev = self.events_rx.recv_async() => match ev {
                    Ok(ConnectionEvent::Close(error_code, reason)) => self.close(error_code, reason),
                    Ok(ConnectionEvent::Proto(ev)) => self.state.lock().unwrap().conn.handle_event(ev),
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
            let mut state = self.state.lock().unwrap();

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
                    ConnectionLost { reason } => state.terminate(reason),
                    _ => {}
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
            self.0.state.lock().unwrap().conn.local_ip()
        }

        /// The peer's UDP address.
        ///
        /// Will panic if called after `poll` has returned `Ready`.
        pub fn remote_address(&self) -> SocketAddr {
            self.0.state.lock().unwrap().conn.remote_address()
        }

        /// Current best estimate of this connection's latency (round-trip-time).
        pub fn rtt(&self) -> Duration {
            self.0.state.lock().unwrap().conn.rtt()
        }

        /// Connection statistics.
        pub fn stats(&self) -> ConnectionStats {
            self.0.state.lock().unwrap().conn.stats()
        }

        /// Current state of the congestion control algorithm. (For debugging
        /// purposes)
        pub fn congestion_state(&self) -> Box<dyn Controller> {
            self.0
                .state
                .lock()
                .unwrap()
                .conn
                .congestion_state()
                .clone_box()
        }

        /// Cryptographic identity of the peer.
        pub fn peer_identity(
            &self,
        ) -> Option<Box<Vec<rustls::pki_types::CertificateDer<'static>>>> {
            self.0
                .state
                .lock()
                .unwrap()
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
                .state
                .lock()
                .unwrap()
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
        inner.state.lock().unwrap().worker = Some(worker);
        Self(inner)
    }

    /// Parameters negotiated during the handshake.
    pub async fn handshake_data(&mut self) -> Result<Box<HandshakeData>, ConnectionError> {
        future::poll_fn(|cx| {
            let mut state = self.0.state.lock().unwrap();
            if let Some(res) = state.try_handshake_data() {
                return Poll::Ready(res);
            }

            match &state.on_handshake_data {
                Some(waker) if waker.will_wake(cx.waker()) => {}
                _ => state.on_handshake_data = Some(cx.waker().clone()),
            }

            if let Some(res) = state.try_handshake_data() {
                Poll::Ready(res)
            } else {
                Poll::Pending
            }
        })
        .await
    }
}

impl Future for Connecting {
    type Output = Result<Connection, ConnectionError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.0.state.lock().unwrap();

        if let Some(res) =
            state.try_map(|state| state.connected.then(|| Connection(self.0.clone())))
        {
            return Poll::Ready(res);
        }

        match &state.on_connected {
            Some(waker) if waker.will_wake(cx.waker()) => {}
            _ => state.on_connected = Some(cx.waker().clone()),
        }

        if let Some(res) =
            state.try_map(|state| state.connected.then(|| Connection(self.0.clone())))
        {
            Poll::Ready(res)
        } else {
            Poll::Pending
        }
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
        self.0.state.lock().unwrap().try_handshake_data().unwrap()
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
        self.0.state.lock().unwrap().conn.datagrams().max_size()
    }

    /// Bytes available in the outgoing datagram buffer.
    ///
    /// When greater than zero, calling [`send_datagram()`](Self::send_datagram)
    /// with a datagram of at most this size is guaranteed not to cause older
    /// datagrams to be dropped.
    pub fn datagram_send_buffer_space(&self) -> usize {
        self.0
            .state
            .lock()
            .unwrap()
            .conn
            .datagrams()
            .send_buffer_space()
    }

    /// Modify the number of remotely initiated unidirectional streams that may
    /// be concurrently open
    ///
    /// No streams may be opened by the peer unless fewer than `count` are
    /// already open. Large `count`s increase both minimum and worst-case
    /// memory consumption.
    pub fn set_max_concurrent_uni_streams(&self, count: VarInt) {
        let mut state = self.0.state.lock().unwrap();
        state.conn.set_max_concurrent_streams(Dir::Uni, count);
        // May need to send MAX_STREAMS to make progress
        state.wake();
    }

    /// See [`quinn_proto::TransportConfig::receive_window()`]
    pub fn set_receive_window(&self, receive_window: VarInt) {
        let mut state = self.0.state.lock().unwrap();
        state.conn.set_receive_window(receive_window);
        state.wake();
    }

    /// Modify the number of remotely initiated bidirectional streams that may
    /// be concurrently open
    ///
    /// No streams may be opened by the peer unless fewer than `count` are
    /// already open. Large `count`s increase both minimum and worst-case
    /// memory consumption.
    pub fn set_max_concurrent_bi_streams(&self, count: VarInt) {
        let mut state = self.0.state.lock().unwrap();
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

    /// Wait for the connection to be closed for any reason
    pub async fn closed(&self) -> ConnectionError {
        let worker = self.0.state.lock().unwrap().worker.take();
        if let Some(worker) = worker {
            let _ = worker.await;
        }

        self.0.state.lock().unwrap().error.clone().unwrap()
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
