//! Builder and other building utils

use std::{borrow::Cow, cell::RefCell, convert::Infallible, marker::PhantomData, rc::Rc};

use quiche::Config;

use super::{error::QuicResult, session::SessionStorage, Inner, SharedInner};
use crate::{QuicClient, QuicServer, ToSocketAddrsAsync};

trait Build {
    fn build(inner: SharedInner) -> Self;
}

impl Build for QuicServer {
    fn build(inner: SharedInner) -> Self {
        Self { inner }
    }
}

impl Build for QuicClient {
    fn build(inner: SharedInner) -> Self {
        Self { inner }
    }
}

/// Marker for building a [`QuicServer`] or [`QuicClient`].
///
/// This trait is sealed, and has only two implementors: [`Server`] and
/// [`Client`].
pub trait Roll {
    /// The type of the quic socket
    #[allow(private_bounds)]
    type Quic: Build;

    /// Whether the socket is a server socket
    const IS_SERVER: bool;
}

/// Marker for building a [`QuicServer`]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Server(Infallible);

/// Marker for building a [`QuicClient`]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Client(Infallible);

impl Roll for Server {
    type Quic = QuicServer;

    const IS_SERVER: bool = true;
}

impl Roll for Client {
    type Quic = QuicClient;

    const IS_SERVER: bool = false;
}

/// A builder for [`QuicServer`]
pub type ServerBuilder<'a, L = (), R = (), S = ()> = Builder<'a, Server, L, R, S>;

/// A builder for [`QuicClient`]
pub type ClientBuilder<'a, L = (), R = (), S = ()> = Builder<'a, Client, L, R, S>;

/// A builder for [`QuicServer`] or [`QuicClient`]
pub struct Builder<'a, Ro, L = (), R = (), S = ()> {
    pub(super) config: Config,
    pub(super) server_name: Option<Cow<'a, str>>,
    pub(super) buf_size: usize,
    pub(super) tuple: (L, R),
    pub(super) session_storage: S,
    pub(super) _roll: std::marker::PhantomData<Ro>,
}

impl<'a, R> Builder<'a, R> {
    /// Create a new QUIC socket builder with given [`Roll`].
    pub fn new<Ro: Roll>() -> QuicResult<Builder<'a, Ro>> {
        let this = Builder {
            tuple: ((), ()),
            config: Config::new(quiche::PROTOCOL_VERSION)?,
            server_name: None,
            buf_size: u16::MAX as usize,
            session_storage: (),
            _roll: PhantomData,
        };

        Ok(this)
    }
}

impl<'a, Ro, L, R, S> Builder<'a, Ro, L, R, S> {
    /// Set the bind address.
    pub fn bind<A: ToSocketAddrsAsync>(self, addr: A) -> Builder<'a, Ro, A, R, S> {
        let Builder {
            tuple,
            config,
            server_name,
            buf_size,
            session_storage,
            _roll,
        } = self;

        Builder {
            tuple: (addr, tuple.1),
            config,
            server_name,
            buf_size,
            session_storage,
            _roll,
        }
    }

    /// Set the remote address.
    pub fn remote<A: ToSocketAddrsAsync>(self, addr: A) -> Builder<'a, Ro, L, A, S> {
        let Builder {
            tuple,
            config,
            server_name,
            buf_size,
            session_storage,
            _roll,
        } = self;

        Builder {
            tuple: (tuple.0, addr),
            config,
            server_name,
            buf_size,
            session_storage,
            _roll,
        }
    }

    /// Set the session storage.
    ///
    /// See [`SessionStorage`] for more information.
    pub fn session_storage<S2: SessionStorage>(self, storage: S2) -> Builder<'a, Ro, L, R, S2> {
        let Builder {
            config,
            server_name,
            buf_size,
            tuple,
            _roll,
            ..
        } = self;

        Builder {
            config,
            server_name,
            buf_size,
            tuple,
            session_storage: storage,
            _roll,
        }
    }

    /// Set the quic config.
    ///
    /// See [`quiche::Config`] for more information.
    pub fn quic_config(mut self, config: Config) -> Self {
        self.config = config;
        self
    }

    /// Set the server name.
    pub fn server_name(mut self, name: impl Into<Cow<'a, str>>) -> Self {
        self.server_name = Some(name.into());
        self
    }

    pub(crate) fn split_ss(self) -> (S, Builder<'a, Ro, L, R>) {
        let new = Builder {
            config: self.config,
            server_name: self.server_name,
            buf_size: self.buf_size,
            tuple: self.tuple,
            _roll: self._roll,
            session_storage: (),
        };

        (self.session_storage, new)
    }
}

impl<'a, Ro, L, R> Builder<'a, Ro, L, R>
where
    Ro: Roll,
    L: ToSocketAddrsAsync,
    R: ToSocketAddrsAsync,
{
    /// Finalize the builder and create a [`QuicServer`] or [`QuicClient`]
    /// depends on the [`Roll`].
    pub async fn build(self) -> QuicResult<Ro::Quic> {
        let inner = Inner::new(self).await?;
        let inner = SharedInner(Rc::new(RefCell::new(inner)));
        inner.clone().spawn().detach();
        Ok(Ro::Quic::build(inner))
    }
}

impl<'a, Ro, L, R, S> Builder<'a, Ro, L, R, S>
where
    Ro: Roll,
    L: ToSocketAddrsAsync,
    R: ToSocketAddrsAsync,
    S: SessionStorage,
{
    /// Finalize the builder and create a [`QuicServer`] or [`QuicClient`] with
    /// session storage depends on the [`Roll`].
    pub async fn build(self) -> QuicResult<Ro::Quic> {
        let inner = Inner::new_with_session(self).await?;
        let inner = SharedInner(Rc::new(RefCell::new(inner)));
        inner.clone().spawn().detach();
        Ok(Ro::Quic::build(inner))
    }
}
