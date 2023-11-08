//! QUIC implementation based on [quiche].

use std::{
    cell::{Cell, RefCell, RefMut},
    collections::BTreeMap,
    net::SocketAddr,
    rc::Rc,
    time::Instant,
};

use compio_buf::{buf_try, BufResult, IntoInner, IoBuf};
use compio_runtime::{event::EventHandle, Task};
use quiche::{Config, Connection, ConnectionId, Error as QuicheError, RecvInfo, SendInfo};

pub mod builder;
mod error;
mod session;
mod split;
pub mod stream;

#[doc(inline)]
pub use {
    builder::{ClientBuilder, ServerBuilder},
    error::QuicResult,
    session::SessionStorage,
    stream::*,
};

use self::{
    builder::{Builder, Roll},
    session::DynSessionStorage,
};
use crate::{ToSocketAddrsAsync, UdpSocket};

// TODO: Use random generated connection id
thread_local! {
    static CON_ID: Cell<u64> = Cell::new(0);
}

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

/// A QUIC server
pub struct QuicServer {
    inner: SharedInner,
}

/// A QUIC client
pub struct QuicClient {
    inner: SharedInner,
}

/// Shared methods for both client and server sockets
macro_rules! socket_fn {
    ($name:literal) => {
        #[doc = concat!("Create a new QUIC ", $name, " socket.")]
        pub async fn new(
            bind: impl ToSocketAddrsAsync,
            remote: impl ToSocketAddrsAsync,
        ) -> QuicResult<Self> {
            Self::builder()?.bind(bind).remote(remote).build().await
        }

        #[doc = concat!("Create a new ", $name, "-initiated bidirectional stream")]
        pub fn bi_stream(&self) -> QuicResult<BiStream> {
            let id = self.inner.next_stream_id().to_bi();
            BiStream::new(self.inner.clone(), id)
        }

        #[doc = concat!("Create a new ", $name, "-initiated unidirectional stream")]
        pub fn uni_stream(&self) -> QuicResult<UniStream> {
            let id = self.inner.next_stream_id().to_uni();
            UniStream::new(self.inner.clone(), id)
        }
    };
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

    /// Create a new QUIC client socket builder.
    pub fn builder() -> QuicResult<ClientBuilder<'static>> {
        ClientBuilder::new()
    }
}

struct Inner {
    buf: Vec<u8>,
    quic: Connection,
    sockets: BTreeMap<FourTuple, UdpSocket>,
    streams: BTreeMap<StreamId, EventHandle>,
    is_server: bool,
    next_stream_id: StreamId,
    config: Config,
    session_storage: Option<Box<dyn DynSessionStorage>>,
    // TODO: add termination machenism
}

impl Inner {
    async fn new<Ro, L, R>(builder: Builder<'_, Ro, L, R, ()>) -> QuicResult<Self>
    where
        Ro: Roll,
        L: ToSocketAddrsAsync,
        R: ToSocketAddrsAsync,
    {
        let Builder {
            tuple,
            mut config,
            buf_size,
            server_name,
            ..
        } = builder;

        let id = CON_ID.with(|x| {
            let id = x.get();
            x.set(id + 1);
            id
        });

        let socket = UdpSocket::bind(tuple.0).await?;
        socket.connect(tuple.1).await?;

        let (local, peer) = (socket.local_addr()?, socket.peer_addr()?);

        let quic = if Ro::IS_SERVER {
            quiche::accept(
                &ConnectionId::from_ref(&id.to_be_bytes()),
                None,
                local,
                peer,
                &mut config,
            )?
        } else {
            quiche::connect(
                server_name.as_deref(),
                &ConnectionId::from_ref(&id.to_be_bytes()),
                local,
                peer,
                &mut config,
            )?
        };

        Ok(Self {
            buf: vec![0; buf_size],
            quic,
            sockets: BTreeMap::from([(FourTuple { local, peer }, socket)]),
            streams: BTreeMap::new(),
            is_server: Ro::IS_SERVER,
            next_stream_id: StreamId::new_bi(0, Ro::IS_SERVER),
            config,
            session_storage: None,
        })
    }

    async fn new_with_session<Ro, L, R, S>(builder: Builder<'_, Ro, L, R, S>) -> QuicResult<Self>
    where
        Ro: Roll,
        L: ToSocketAddrsAsync,
        R: ToSocketAddrsAsync,
        S: SessionStorage,
    {
        let (mut session_storage, builder) = builder.split_ss();
        let session = session_storage.retrieve_session().await?;
        let mut this = Self::new(builder).await?;
        if let Some(session) = session {
            this.quic.set_session(&session)?;
        }
        this.session_storage = Some(session_storage.into());
        Ok(this)
    }
}

#[derive(Clone)]
struct SharedInner(Rc<RefCell<Inner>>);

impl SharedInner {
    fn get(&self) -> RefMut<'_, Inner> {
        self.0.borrow_mut()
    }

    fn with<R>(&self, f: impl FnOnce(&mut Inner) -> R) -> R {
        f(&mut self.0.borrow_mut())
    }

    fn take<T: Default>(&self, f: impl FnOnce(&mut Inner) -> &mut T) -> T {
        std::mem::take(f(&mut self.0.borrow_mut()))
    }

    fn next_stream_id(&self) -> StreamId {
        self.with(|s| s.next_stream_id.into_next())
    }

    fn spawn(self) -> Task<QuicResult<()>> {
        compio_runtime::spawn(async move {
            let err = loop {
                if let Err(e) = self.tick().await {
                    break e;
                }
            };

            self.clean_up().await;

            Err(err)
        })
    }

    async fn clean_up(&self) {
        if let Err(QuicheError::Done) = self.with(|s| s.quic.close(false, 0x00, b"")) {
            return;
        }

        while self.with(|x| !x.quic.is_closed()) {
            if self.tick().await.is_err() {
                break;
            }
        }

        for socket in self.take(|s| &mut s.sockets).into_values() {
            socket.close().await.ok();
        }
    }

    async fn tick(&self) -> QuicResult<()> {
        self.recv().await?;
        self.send().await?;

        let timeout = self.with(|s| s.quic.timeout());

        if let Some(timeout) = timeout {
            compio_runtime::time::sleep(timeout).await;
            self.with(|s| s.quic.on_timeout());
            self.send().await?;
        }

        Ok(())
    }

    async fn send(&self) -> QuicResult<()> {
        if self.with(|s| s.quic.is_draining()) {
            return Ok(());
        }

        loop {
            let res = self.with(|Inner { quic, buf, .. }| {
                buf.clear();
                quic.send(buf) // Read the packet to send
            });
            let (len, info) = match res {
                Ok(res) => res,
                Err(QuicheError::Done) => break,
                Err(e) => {
                    return Err(e.into());
                }
            };

            if info.at > Instant::now() {
                compio_runtime::time::sleep_until(info.at).await;
            }

            let tuple = info.into();

            let socket = self.with(|s| s.sockets.remove(&tuple).expect("socket not found"));
            let mut buf = self.take(|s| &mut s.buf);
            let mut progress = 0;

            buf.clear();

            while progress < len {
                (progress, buf) = buf_try! {
                    @try socket.send(buf.slice(progress..len))
                        .await
                        .into_inner()
                        .map_res(|sent| sent + progress)
                };
            }

            self.with(|s| {
                s.buf = buf;
                s.sockets.insert(tuple, socket);
            })
        }

        Ok(())
    }

    async fn recv(&self) -> QuicResult<()> {
        let sockets = self.take(|s| &mut s.sockets);
        let mut buf = self.take(|s| &mut s.buf);
        let mut len;

        for (tuple, socket) in sockets.iter() {
            buf.clear();
            (len, buf) = match socket.recv(buf).await {
                BufResult(Err(_), b) => {
                    buf = b;
                    continue;
                }
                BufResult(Ok(0), b) => {
                    buf = b;
                    continue;
                }
                BufResult(Ok(len), buf) => (len, buf),
            };
            self.with(|s| s.quic.recv(&mut buf[..len], tuple.into()))?;
        }

        self.with(|s| {
            s.buf = buf;
            s.sockets = sockets;
        });

        Ok(())
    }

    fn wake_stream(&self, id: StreamId) {
        self.with(|s| {
            let Some(handle) = s.streams.get(&id) else {
                return;
            };

            // When the stream is closed, remove it from the map
            if handle.notify().is_err() {
                s.streams.remove(&id);
            }
        })
    }
}
