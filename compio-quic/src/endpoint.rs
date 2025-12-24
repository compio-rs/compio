use std::{
    collections::VecDeque,
    io,
    mem::ManuallyDrop,
    net::{SocketAddr, SocketAddrV6},
    ops::Deref,
    pin::pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
    time::Instant,
};

use compio_buf::{BufResult, bytes::Bytes};
use compio_log::{Instrument, error};
#[cfg(rustls)]
use compio_net::ToSocketAddrsAsync;
use compio_net::UdpSocket;
use compio_runtime::JoinHandle;
use flume::{Receiver, Sender, unbounded};
use futures_util::{
    FutureExt, StreamExt,
    future::{self},
    select,
    task::AtomicWaker,
};
use quinn_proto::{
    ClientConfig, ConnectError, ConnectionError, ConnectionHandle, DatagramEvent, EndpointConfig,
    EndpointEvent, ServerConfig, Transmit, VarInt,
};
use rustc_hash::FxHashMap as HashMap;

use crate::{Connecting, ConnectionEvent, Incoming, RecvMeta, Socket};

#[derive(Debug)]
struct EndpointState {
    endpoint: quinn_proto::Endpoint,
    worker: Option<JoinHandle<()>>,
    connections: HashMap<ConnectionHandle, Sender<ConnectionEvent>>,
    close: Option<(VarInt, Bytes)>,
    exit_on_idle: bool,
    incoming: VecDeque<quinn_proto::Incoming>,
    incoming_wakers: VecDeque<Waker>,
}

impl EndpointState {
    fn handle_data(&mut self, meta: RecvMeta, buf: &[u8], respond_fn: impl Fn(Vec<u8>, Transmit)) {
        let now = Instant::now();
        for data in buf[..meta.len]
            .chunks(meta.stride.min(meta.len))
            .map(Into::into)
        {
            let mut resp_buf = Vec::new();
            match self.endpoint.handle(
                now,
                meta.remote,
                meta.local_ip,
                meta.ecn,
                data,
                &mut resp_buf,
            ) {
                Some(DatagramEvent::NewConnection(incoming)) => {
                    if self.close.is_none() {
                        self.incoming.push_back(incoming);
                    } else {
                        let transmit = self.endpoint.refuse(incoming, &mut resp_buf);
                        respond_fn(resp_buf, transmit);
                    }
                }
                Some(DatagramEvent::ConnectionEvent(ch, event)) => {
                    let _ = self
                        .connections
                        .get(&ch)
                        .unwrap()
                        .send(ConnectionEvent::Proto(event));
                }
                Some(DatagramEvent::Response(transmit)) => respond_fn(resp_buf, transmit),
                None => {}
            }
        }
    }

    fn handle_event(&mut self, ch: ConnectionHandle, event: EndpointEvent) {
        if event.is_drained() {
            self.connections.remove(&ch);
        }
        if let Some(event) = self.endpoint.handle_event(ch, event) {
            let _ = self
                .connections
                .get(&ch)
                .unwrap()
                .send(ConnectionEvent::Proto(event));
        }
    }

    fn is_idle(&self) -> bool {
        self.connections.is_empty()
    }

    fn poll_incoming(&mut self, cx: &mut Context) -> Poll<Option<quinn_proto::Incoming>> {
        if self.close.is_none() {
            if let Some(incoming) = self.incoming.pop_front() {
                Poll::Ready(Some(incoming))
            } else {
                self.incoming_wakers.push_back(cx.waker().clone());
                Poll::Pending
            }
        } else {
            Poll::Ready(None)
        }
    }

    fn new_connection(
        &mut self,
        handle: ConnectionHandle,
        conn: quinn_proto::Connection,
        socket: Socket,
        events_tx: Sender<(ConnectionHandle, EndpointEvent)>,
    ) -> Connecting {
        let (tx, rx) = unbounded();
        if let Some((error_code, reason)) = &self.close {
            tx.send(ConnectionEvent::Close(*error_code, reason.clone()))
                .unwrap();
        }
        self.connections.insert(handle, tx);
        Connecting::new(handle, conn, socket, events_tx, rx)
    }
}

type ChannelPair<T> = (Sender<T>, Receiver<T>);

#[derive(Debug)]
pub(crate) struct EndpointInner {
    state: Mutex<EndpointState>,
    socket: Socket,
    ipv6: bool,
    events: ChannelPair<(ConnectionHandle, EndpointEvent)>,
    done: AtomicWaker,
}

impl EndpointInner {
    fn new(
        socket: UdpSocket,
        config: EndpointConfig,
        server_config: Option<ServerConfig>,
    ) -> io::Result<Self> {
        let socket = Socket::new(socket)?;
        let ipv6 = socket.local_addr()?.is_ipv6();
        let allow_mtud = !socket.may_fragment();

        Ok(Self {
            state: Mutex::new(EndpointState {
                endpoint: quinn_proto::Endpoint::new(
                    Arc::new(config),
                    server_config.map(Arc::new),
                    allow_mtud,
                    None,
                ),
                worker: None,
                connections: HashMap::default(),
                close: None,
                exit_on_idle: false,
                incoming: VecDeque::new(),
                incoming_wakers: VecDeque::new(),
            }),
            socket,
            ipv6,
            events: unbounded(),
            done: AtomicWaker::new(),
        })
    }

    fn connect(
        &self,
        remote: SocketAddr,
        server_name: &str,
        config: ClientConfig,
    ) -> Result<Connecting, ConnectError> {
        let mut state = self.state.lock().unwrap();

        if state.worker.is_none() {
            return Err(ConnectError::EndpointStopping);
        }
        if remote.is_ipv6() && !self.ipv6 {
            return Err(ConnectError::InvalidRemoteAddress(remote));
        }
        let remote = if self.ipv6 {
            SocketAddr::V6(match remote {
                SocketAddr::V4(addr) => {
                    SocketAddrV6::new(addr.ip().to_ipv6_mapped(), addr.port(), 0, 0)
                }
                SocketAddr::V6(addr) => addr,
            })
        } else {
            remote
        };

        let (handle, conn) = state
            .endpoint
            .connect(Instant::now(), config, remote, server_name)?;

        Ok(state.new_connection(handle, conn, self.socket.clone(), self.events.0.clone()))
    }

    fn respond(&self, buf: Vec<u8>, transmit: Transmit) {
        let socket = self.socket.clone();
        compio_runtime::spawn(async move {
            socket.send(buf, &transmit).await;
        })
        .detach();
    }

    pub(crate) fn accept(
        &self,
        incoming: quinn_proto::Incoming,
        server_config: Option<ServerConfig>,
    ) -> Result<Connecting, ConnectionError> {
        let mut state = self.state.lock().unwrap();
        let mut resp_buf = Vec::new();
        let now = Instant::now();
        match state
            .endpoint
            .accept(incoming, now, &mut resp_buf, server_config.map(Arc::new))
        {
            Ok((handle, conn)) => {
                Ok(state.new_connection(handle, conn, self.socket.clone(), self.events.0.clone()))
            }
            Err(err) => {
                if let Some(transmit) = err.response {
                    self.respond(resp_buf, transmit);
                }
                Err(err.cause)
            }
        }
    }

    pub(crate) fn refuse(&self, incoming: quinn_proto::Incoming) {
        let mut state = self.state.lock().unwrap();
        let mut resp_buf = Vec::new();
        let transmit = state.endpoint.refuse(incoming, &mut resp_buf);
        self.respond(resp_buf, transmit);
    }

    #[allow(clippy::result_large_err)]
    pub(crate) fn retry(
        &self,
        incoming: quinn_proto::Incoming,
    ) -> Result<(), quinn_proto::RetryError> {
        let mut state = self.state.lock().unwrap();
        let mut resp_buf = Vec::new();
        let transmit = state.endpoint.retry(incoming, &mut resp_buf)?;
        self.respond(resp_buf, transmit);
        Ok(())
    }

    pub(crate) fn ignore(&self, incoming: quinn_proto::Incoming) {
        let mut state = self.state.lock().unwrap();
        state.endpoint.ignore(incoming);
    }

    async fn run(&self) -> io::Result<()> {
        let respond_fn = |buf: Vec<u8>, transmit: Transmit| self.respond(buf, transmit);

        let mut recv_fut = pin!(
            self.socket
                .recv(Vec::with_capacity(
                    self.state
                        .lock()
                        .unwrap()
                        .endpoint
                        .config()
                        .get_max_udp_payload_size()
                        .min(64 * 1024) as usize
                        * self.socket.max_gro_segments(),
                ))
                .fuse()
        );

        let mut event_stream = self.events.1.stream().ready_chunks(100);

        loop {
            let mut state = select! {
                BufResult(res, recv_buf) = recv_fut => {
                    let mut state = self.state.lock().unwrap();
                    match res {
                        Ok(meta) => state.handle_data(meta, &recv_buf, respond_fn),
                        Err(e) if e.kind() == io::ErrorKind::ConnectionReset => {}
                        #[cfg(windows)]
                        Err(e) if e.raw_os_error() == Some(windows_sys::Win32::Foundation::ERROR_PORT_UNREACHABLE as _) => {}
                        Err(e) => break Err(e),
                    }
                    recv_fut.set(self.socket.recv(recv_buf).fuse());
                    state
                },
                events = event_stream.select_next_some() => {
                    let mut state = self.state.lock().unwrap();
                    for (ch, event) in events {
                        state.handle_event(ch, event);
                    }
                    state
                },
            };

            if state.exit_on_idle && state.is_idle() {
                break Ok(());
            }
            if !state.incoming.is_empty() {
                let n = state.incoming.len().min(state.incoming_wakers.len());
                state.incoming_wakers.drain(..n).for_each(Waker::wake);
            }
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct EndpointRef(Arc<EndpointInner>);

impl EndpointRef {
    // Modified from [`SharedFd::try_unwrap_inner`], see notes there.
    unsafe fn try_unwrap_inner(&self) -> Option<EndpointInner> {
        let ptr = unsafe { std::ptr::read(&self.0) };
        match Arc::try_unwrap(ptr) {
            Ok(inner) => Some(inner),
            Err(ptr) => {
                std::mem::forget(ptr);
                None
            }
        }
    }

    async fn shutdown(self) -> io::Result<()> {
        let (worker, idle) = {
            let mut state = self.0.state.lock().unwrap();
            let idle = state.is_idle();
            if !idle {
                state.exit_on_idle = true;
            }
            (state.worker.take(), idle)
        };
        if let Some(worker) = worker {
            if idle {
                worker.cancel().await;
            } else {
                let _ = worker.await;
            }
        }

        let this = ManuallyDrop::new(self);
        let inner = future::poll_fn(move |cx| {
            if let Some(inner) = unsafe { Self::try_unwrap_inner(&this) } {
                return Poll::Ready(inner);
            }

            this.done.register(cx.waker());

            if let Some(inner) = unsafe { Self::try_unwrap_inner(&this) } {
                Poll::Ready(inner)
            } else {
                Poll::Pending
            }
        })
        .await;

        inner.socket.close().await
    }
}

impl Drop for EndpointRef {
    fn drop(&mut self) {
        if Arc::strong_count(&self.0) == 2 {
            // There are actually two cases:
            // 1. User is trying to shutdown the socket.
            self.0.done.wake();
            // 2. User dropped the endpoint but the worker is still running.
            self.0.state.lock().unwrap().exit_on_idle = true;
        }
    }
}

impl Deref for EndpointRef {
    type Target = EndpointInner;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// A QUIC endpoint.
#[derive(Debug, Clone)]
pub struct Endpoint {
    inner: EndpointRef,
    /// The client configuration used by `connect`
    pub default_client_config: Option<ClientConfig>,
}

impl Endpoint {
    /// Create a QUIC endpoint.
    pub fn new(
        socket: UdpSocket,
        config: EndpointConfig,
        server_config: Option<ServerConfig>,
        default_client_config: Option<ClientConfig>,
    ) -> io::Result<Self> {
        let inner = EndpointRef(Arc::new(EndpointInner::new(socket, config, server_config)?));
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
        inner.state.lock().unwrap().worker = Some(worker);
        Ok(Self {
            inner,
            default_client_config,
        })
    }

    /// Helper to construct an endpoint for use with outgoing connections only.
    ///
    /// Note that `addr` is the *local* address to bind to, which should usually
    /// be a wildcard address like `0.0.0.0:0` or `[::]:0`, which allow
    /// communication with any reachable IPv4 or IPv6 address respectively
    /// from an OS-assigned port.
    ///
    /// If an IPv6 address is provided, the socket may dual-stack depending on
    /// the platform, so as to allow communication with both IPv4 and IPv6
    /// addresses. As such, calling this method with the address `[::]:0` is a
    /// reasonable default to maximize the ability to connect to other
    /// address.
    ///
    /// IPv4 client is never dual-stack.
    #[cfg(rustls)]
    pub async fn client(addr: impl ToSocketAddrsAsync) -> io::Result<Endpoint> {
        // TODO: try to enable dual-stack on all platforms, notably Windows
        let socket = UdpSocket::bind(addr).await?;
        Self::new(socket, EndpointConfig::default(), None, None)
    }

    /// Helper to construct an endpoint for use with both incoming and outgoing
    /// connections
    ///
    /// Platform defaults for dual-stack sockets vary. For example, any socket
    /// bound to a wildcard IPv6 address on Windows will not by default be
    /// able to communicate with IPv4 addresses. Portable applications
    /// should bind an address that matches the family they wish to
    /// communicate within.
    #[cfg(rustls)]
    pub async fn server(addr: impl ToSocketAddrsAsync, config: ServerConfig) -> io::Result<Self> {
        let socket = UdpSocket::bind(addr).await?;
        Self::new(socket, EndpointConfig::default(), Some(config), None)
    }

    /// Connect to a remote endpoint.
    pub fn connect(
        &self,
        remote: SocketAddr,
        server_name: &str,
        config: Option<ClientConfig>,
    ) -> Result<Connecting, ConnectError> {
        let config = config
            .or_else(|| self.default_client_config.clone())
            .ok_or(ConnectError::NoDefaultClientConfig)?;

        self.inner.connect(remote, server_name, config)
    }

    /// Wait for the next incoming connection attempt from a client.
    ///
    /// Yields [`Incoming`]s, or `None` if the endpoint is
    /// [`close`](Self::close)d. [`Incoming`] can be `await`ed to obtain the
    /// final [`Connection`](crate::Connection), or used to e.g. filter
    /// connection attempts or force address validation, or converted into an
    /// intermediate `Connecting` future which can be used to e.g. send 0.5-RTT
    /// data.
    pub async fn wait_incoming(&self) -> Option<Incoming> {
        future::poll_fn(|cx| self.inner.state.lock().unwrap().poll_incoming(cx))
            .await
            .map(|incoming| Incoming::new(incoming, self.inner.clone()))
    }

    /// Replace the server configuration, affecting new incoming connections
    /// only.
    ///
    /// Useful for e.g. refreshing TLS certificates without disrupting existing
    /// connections.
    pub fn set_server_config(&self, server_config: Option<ServerConfig>) {
        self.inner
            .state
            .lock()
            .unwrap()
            .endpoint
            .set_server_config(server_config.map(Arc::new))
    }

    /// Get the local `SocketAddr` the underlying socket is bound to.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.socket.local_addr()
    }

    /// Get the number of connections that are currently open.
    pub fn open_connections(&self) -> usize {
        self.inner.state.lock().unwrap().endpoint.open_connections()
    }

    /// Close all of this endpoint's connections immediately and cease accepting
    /// new connections.
    ///
    /// See [`Connection::close()`] for details.
    ///
    /// [`Connection::close()`]: crate::Connection::close
    pub fn close(&self, error_code: VarInt, reason: &[u8]) {
        let reason = Bytes::copy_from_slice(reason);
        let mut state = self.inner.state.lock().unwrap();
        if state.close.is_some() {
            return;
        }
        state.close = Some((error_code, reason.clone()));
        for conn in state.connections.values() {
            let _ = conn.send(ConnectionEvent::Close(error_code, reason.clone()));
        }
        state.incoming_wakers.drain(..).for_each(Waker::wake);
    }

    /// Gracefully shutdown the endpoint.
    ///
    /// Wait for all connections on the endpoint to be cleanly shut down and
    /// close the underlying socket. This will wait for all clones of the
    /// endpoint, all connections and all streams to be dropped before
    /// closing the socket.
    ///
    /// Waiting for this condition before exiting ensures that a good-faith
    /// effort is made to notify peers of recent connection closes, whereas
    /// exiting immediately could force them to wait out the idle timeout
    /// period.
    ///
    /// Does not proactively close existing connections. Consider calling
    /// [`close()`] if that is desired.
    ///
    /// [`close()`]: Endpoint::close
    pub async fn shutdown(self) -> io::Result<()> {
        self.inner.shutdown().await
    }
}
