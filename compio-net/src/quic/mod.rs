//! QUIC implementation based on [quiche].

use std::{
    cell::{RefCell, RefMut},
    collections::BTreeMap,
    future::Future,
    io,
    net::SocketAddr,
    ops::RangeBounds,
    rc::Rc,
};

use compio_buf::{BufResult, IntoInner, IoBuf};
use compio_runtime::Task;
use quiche::{Config, Header, RecvInfo, SendInfo, Type};

pub mod builder;
mod connection;
mod endpoint;
mod error;
mod session;
mod split;
pub mod stream;

#[doc(inline)]
pub use {
    builder::{ClientBuilder, ServerBuilder},
    connection::{Connection, ConnectionId},
    error::QuicResult,
    session::SessionStorage,
    stream::{BiStream, StreamId, UniStream},
};

use self::{builder::Builder, session::DynSessionStorage};
use crate::{
    quic::{
        builder::{Client, Server},
        connection::ConnInner,
        error::{IoResult, QuicError},
    },
    ToSocketAddrsAsync, UdpSocket,
};

/// A 4-tuple of (source IP, source port, destination IP, destination port).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct FourTuple {
    pub local: SocketAddr,
    pub peer: SocketAddr,
}

impl From<SendInfo> for FourTuple {
    fn from(info: SendInfo) -> Self {
        Self {
            local: info.from,
            peer: info.to,
        }
    }
}

impl From<&FourTuple> for RecvInfo {
    fn from(tuple: &FourTuple) -> RecvInfo {
        RecvInfo {
            from: tuple.local,
            to: tuple.peer,
        }
    }
}

impl From<FourTuple> for RecvInfo {
    fn from(tuple: FourTuple) -> RecvInfo {
        RecvInfo {
            from: tuple.local,
            to: tuple.peer,
        }
    }
}

/// A QUIC server
pub struct QuicServer {
    inner: Endpoint,
}

/// A QUIC client
pub struct QuicClient {
    inner: Endpoint,
}

/// Shared methods for both client and server sockets
macro_rules! socket_fn {
    ($name:literal) => {};
}

impl QuicServer {
    socket_fn!("server");

    /// Create a new QUIC server socket builder.
    pub fn builder() -> QuicResult<ServerBuilder<'static>> {
        ServerBuilder::new()
    }
}

impl QuicClient {
    socket_fn!("client");

    /// Create a new QUIC client socket with given local and remote address.
    pub async fn new(
        bind: impl ToSocketAddrsAsync,
        remote: impl ToSocketAddrsAsync,
    ) -> QuicResult<Self> {
        Self::builder()?.bind(bind).remote(remote).build().await
    }

    /// Create a new QUIC client socket builder.
    pub fn builder() -> QuicResult<ClientBuilder<'static>> {
        ClientBuilder::new()
    }
}

/// An internally-mutable shared pointer
#[repr(transparent)]
pub(crate) struct Shared<T>(Rc<RefCell<T>>);

impl<T> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> Shared<T> {
    pub(crate) fn new(inner: T) -> Self {
        Self(Rc::new(RefCell::new(inner)))
    }

    pub(crate) fn get(&self) -> RefMut<'_, T> {
        self.0.borrow_mut()
    }

    pub(crate) fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        f(&mut self.get())
    }

    pub(crate) fn take<P>(&self, f: impl FnOnce(&mut T) -> &mut P) -> P
    where
        P: Default,
    {
        std::mem::take(f(&mut self.get()))
    }
}

/// Buffer and socket for sending and receiving
struct Io {
    buf: Vec<u8>,
    socket: Rc<UdpSocket>,
}

struct IoRef<'a> {
    buf: Vec<u8>,
    socket: &'a UdpSocket,
}

macro_rules! impl_io {
    () => {
        async fn recv(self) -> IoResult<(usize, SocketAddr), Self> {
            let Self { buf, socket } = self;
            let BufResult(res, buf) = socket.recv_from(buf).await;
            (res.map_err(Into::into), Self { buf, socket })
        }

        async fn send(
            self,
            range: impl RangeBounds<usize>,
            addr: SocketAddr,
        ) -> IoResult<usize, Self> {
            let Self { buf, socket } = self;
            let BufResult(res, buf) = socket.send_to(buf.slice(range), addr).await.into_inner();
            (res.map_err(Into::into), Self { buf, socket })
        }

        async fn send_all(mut self, len: usize, addr: SocketAddr) -> IoResult<(), Self> {
            let mut progress = 0;
            let mut res;

            while progress < len {
                (res, self) = self.send(progress..len, addr).await;
                match res {
                    Ok(sent) => progress += sent,
                    Err(e) => return (Err(e), self),
                }
            }

            self.buf.clear();

            (Ok(()), self)
        }

        fn clear(&mut self) {
            self.buf.clear();
        }
    };
}

impl Io {
    impl_io!();
}

impl<'a> IoRef<'a> {
    impl_io!();
}

struct EndpointInner {
    local: SocketAddr,
    socket: Rc<UdpSocket>,
    config: Config,
    recv_buf: Vec<u8>,
    connections: BTreeMap<ConnectionId, Connection>,
    next_stream_id: StreamId,
    session_storage: Option<Box<dyn DynSessionStorage>>,
    // TODO: add termination machenism
}

impl EndpointInner {
    async fn new_client<L, R>(builder: Builder<'_, Client, L, R>) -> QuicResult<Self>
    where
        L: ToSocketAddrsAsync,
        R: ToSocketAddrsAsync,
    {
        let Builder {
            tuple,
            mut config,
            server_name,
            ..
        } = builder;

        let socket = Rc::new(UdpSocket::bind(tuple.0).await?);
        let local = socket.local_addr()?;
        let Some(peer) = tuple.1.to_socket_addrs_async().await?.next() else {
            return Err(QuicError::Io(io::Error::new(
                io::ErrorKind::AddrNotAvailable,
                "No peer address found",
            )));
        };

        let tuple = FourTuple { local, peer };
        let (id, inner) = ConnInner::connect(server_name.as_deref(), tuple, &mut config)?;

        Ok(Self {
            local,
            config,
            socket,
            recv_buf: vec![0; 1024],
            connections: BTreeMap::from_iter([(id, Connection::new(inner))]),
            next_stream_id: StreamId::new_bi(0, false),
            session_storage: None,
        })
    }

    async fn new_server<L>(builder: Builder<'_, Server, L>) -> QuicResult<Self>
    where
        L: ToSocketAddrsAsync,
    {
        let Builder { tuple, config, .. } = builder;

        let socket = Rc::new(UdpSocket::bind(tuple.0).await?);
        let local = socket.local_addr()?;

        Ok(Self {
            local,
            socket,
            config,
            recv_buf: vec![0; 1024],
            connections: Default::default(),
            next_stream_id: StreamId::new_bi(0, false),
            session_storage: None,
        })
    }

    async fn with_session(mut self, s: impl SessionStorage) -> QuicResult<Self> {
        // let session = s.retrieve_session().await?;
        // if let Some(session) = session {
        //     if let Some(quic) = self.quic.as_mut() {
        //         quic.set_session(&session);
        //     }
        // }
        self.session_storage = Some(s.into());
        Ok(self)
    }
}

type Endpoint = Shared<EndpointInner>;

impl Endpoint {
    fn next_stream_id(&self) -> StreamId {
        self.with(|s| s.next_stream_id.into_next())
    }

    async fn with_io<F, R>(&self, func: impl FnOnce(Io) -> F) -> QuicResult<R>
    where
        F: Future<Output = IoResult<R>>,
    {
        let io = Io {
            buf: self.take(|s| &mut s.recv_buf),
            socket: self.with(|s| s.socket.clone()),
        };
        let (res, mut io) = func(io).await;
        io.clear();
        self.with(|x| x.recv_buf = io.buf);
        res
    }

    fn spawn(self) {
        compio_runtime::spawn(self.recv_task()).detach();
    }

    async fn clean_up(&self) {
        // if let Err(QuicheError::Done) = self.with(|s|
        // s.connections.close(false, 0x00, b"")) {     return;
        // }

        // while self.with(|x| !x.connections.is_closed()) {
        //     if self.tick().await.is_err() {
        //         break;
        //     }
        // }

        // self.with(|x| x.socket.close()).await;
    }

    async fn tick(&self) -> QuicResult<()> {
        let socket = self.with(|s| s.socket.clone());
        let con = self.take(|s| &mut s.connections);

        self.with(|s| s.connections = con);

        // let timeout = self.with(|s| s.connections.timeout());

        // if let Some(timeout) = timeout {
        //     compio_runtime::time::sleep(timeout).await;
        //     self.with(|s| s.connections.on_timeout());
        //     self.send().await?;
        // }

        Ok(())
    }

    async fn recv_task(self) -> QuicResult<()> {
        loop {
            let (len, peer) = self.with_io(Io::recv).await?;
            let mut s = self.get();

            let EndpointInner {
                local,
                socket,
                config,
                recv_buf,
                connections,
                ..
            } = &mut *s;

            let buf = &mut recv_buf[..len];
            let tuple = FourTuple {
                local: *local,
                peer,
            };
            let hdr = Header::from_slice(buf, ConnectionId::LEN)?;
            let found = (&hdr.dcid)
                .try_into()
                .ok()
                .and_then(|c: ConnectionId| connections.get(&c));
            let con = if let Some(found) = found {
                found
            } else {
                if hdr.ty != Type::Initial {
                    // Non-initial packets should only be sent to existing connections. Drop the
                    // packet.
                    return Ok(());
                }

                let (id, con) = ConnInner::accept(tuple, config)?;
                let con = Connection::new(con);
                con.spawn(socket.clone());

                connections.insert(id, con);
                connections.get(&id).unwrap()
            };

            con.get().quic.recv(buf, tuple.into())?;
        }
    }
}
