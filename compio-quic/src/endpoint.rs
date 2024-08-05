use std::{
    collections::{HashMap, VecDeque},
    io,
    mem::ManuallyDrop,
    net::{SocketAddr, SocketAddrV6},
    pin::pin,
    sync::{Arc, Mutex},
    task::Poll,
    time::Instant,
};

use compio_buf::BufResult;
use compio_net::UdpSocket;
use compio_runtime::JoinHandle;
use event_listener::{listener, Event, IntoNotification};
use flume::{unbounded, Receiver, Sender};
use futures_util::{
    future::{self},
    select,
    task::AtomicWaker,
    FutureExt,
};
use quinn_proto::{
    ClientConfig, ConnectError, ConnectionError, ConnectionHandle, DatagramEvent, EndpointConfig,
    EndpointEvent, ServerConfig, Transmit, VarInt,
};

use crate::{
    ClientBuilder, Connecting, ConnectionEvent, Incoming, RecvMeta, ServerBuilder, Socket,
};

#[derive(Debug)]
struct EndpointState {
    endpoint: quinn_proto::Endpoint,
    worker: Option<JoinHandle<()>>,
    connections: HashMap<ConnectionHandle, Sender<ConnectionEvent>>,
    close: Option<(VarInt, String)>,
    incoming: VecDeque<quinn_proto::Incoming>,
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

    fn try_get_incoming(&mut self) -> Option<Option<quinn_proto::Incoming>> {
        if self.close.is_none() {
            self.incoming.pop_front().map(Some)
        } else {
            Some(None)
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
    incoming: Event,
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
                connections: HashMap::new(),
                close: None,
                incoming: VecDeque::new(),
            }),
            socket,
            ipv6,
            events: unbounded(),
            done: AtomicWaker::new(),
            incoming: Event::new(),
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
            let _ = socket.send(buf, &transmit).await;
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

        let respond_fn = |buf: Vec<u8>, transmit: Transmit| self.respond(buf, transmit);

        loop {
            select! {
                BufResult(res, recv_buf) = recv_fut => {
                    match res {
                        Ok(meta) => self.state.lock().unwrap().handle_data(meta, &recv_buf, respond_fn),
                        Err(e) if e.kind() == io::ErrorKind::ConnectionReset => {}
                        Err(e) => break Err(e),
                    }
                    recv_fut.set(self.socket.recv(recv_buf).fuse());
                },
                (ch, event) = self.events.1.recv_async().map(Result::unwrap) => {
                    self.state.lock().unwrap().handle_event(ch, event);
                },
            }

            let state = self.state.lock().unwrap();
            if state.close.is_some() && state.is_idle() {
                break Ok(());
            }
            if !state.incoming.is_empty() {
                self.incoming.notify(state.incoming.len().additional());
            }
        }
    }
}

/// A QUIC endpoint.
#[derive(Debug, Clone)]
pub struct Endpoint {
    inner: Arc<EndpointInner>,
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
        let inner = Arc::new(EndpointInner::new(socket, config, server_config)?);
        let worker = compio_runtime::spawn({
            let inner = inner.clone();
            async move { inner.run().await.unwrap() }
        });
        inner.state.lock().unwrap().worker = Some(worker);
        Ok(Self {
            inner,
            default_client_config,
        })
    }

    /// Create a builder for a QUIC client.
    pub fn client() -> ClientBuilder<()> {
        ClientBuilder::default()
    }

    /// Create a builder for a QUIC server.
    pub fn server() -> ServerBuilder<()> {
        ServerBuilder::default()
    }

    /// Connect to a remote endpoint.
    pub fn connect(
        &self,
        remote: SocketAddr,
        server_name: &str,
        config: Option<ClientConfig>,
    ) -> Result<Connecting, ConnectError> {
        let config = if let Some(config) = config {
            config
        } else if let Some(config) = &self.default_client_config {
            config.clone()
        } else {
            return Err(ConnectError::NoDefaultClientConfig);
        };

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
        loop {
            if let Some(incoming) = self.inner.state.lock().unwrap().try_get_incoming() {
                return incoming.map(|incoming| Incoming::new(incoming, self.inner.clone()));
            }

            listener!(self.inner.incoming => listener);

            if let Some(incoming) = self.inner.state.lock().unwrap().try_get_incoming() {
                return incoming.map(|incoming| Incoming::new(incoming, self.inner.clone()));
            }

            listener.await;
        }
    }

    // Modified from [`SharedFd::try_unwrap_inner`], see notes there.
    unsafe fn try_unwrap_inner(this: &ManuallyDrop<Self>) -> Option<EndpointInner> {
        let ptr = ManuallyDrop::new(std::ptr::read(&this.inner));
        match Arc::try_unwrap(ManuallyDrop::into_inner(ptr)) {
            Ok(inner) => Some(inner),
            Err(ptr) => {
                std::mem::forget(ptr);
                None
            }
        }
    }

    /// Shutdown the endpoint and close the underlying socket.
    ///
    /// This will close all connections and the underlying socket. Note that it
    /// will wait for all connections and all clones of the endpoint (and any
    /// clone of the underlying socket) to be dropped before closing the socket.
    ///
    /// If the endpoint has already been closed or is closing, this will return
    /// immediately with `Ok(())`.
    ///
    /// See [`Connection::close()`](crate::Connection::close) for details.
    pub async fn close(self, error_code: VarInt, reason: &str) -> io::Result<()> {
        let reason = reason.to_string();

        {
            let close = &mut self.inner.state.lock().unwrap().close;
            if close.is_some() {
                return Ok(());
            }
            close.replace((error_code, reason.clone()));
        }

        for conn in self.inner.state.lock().unwrap().connections.values() {
            let _ = conn.send(ConnectionEvent::Close(error_code, reason.clone()));
        }

        let worker = self.inner.state.lock().unwrap().worker.take();
        if let Some(worker) = worker {
            if self.inner.state.lock().unwrap().is_idle() {
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

            this.inner.done.register(cx.waker());

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

impl Drop for Endpoint {
    fn drop(&mut self) {
        if Arc::strong_count(&self.inner) == 2 {
            self.inner.done.wake();
        }
    }
}
