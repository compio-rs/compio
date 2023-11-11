use std::{cell::RefCell, collections::BTreeMap, rc::Rc, task::Waker, time::Instant};

use quiche::Error as QuicheError;
use rand::{rngs::ThreadRng, thread_rng};
use uuid7::{Uuid, V7Generator};

use crate::{
    quic::{
        error::{try_io, IoResult, QuicError},
        FourTuple, Io, Shared, StreamId,
    },
    QuicResult, UdpSocket,
};

struct ConnTable {
    connections: BTreeMap<ConnectionId, Connection>,
}

/// A connection id
///
/// Underlying is a time-encoded [`Uuid`], version 7.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct ConnectionId(Uuid);

impl TryFrom<quiche::ConnectionId<'_>> for ConnectionId {
    type Error = QuicError;

    fn try_from(value: quiche::ConnectionId<'_>) -> Result<Self, Self::Error> {
        Self::try_from(&value)
    }
}

impl TryFrom<&quiche::ConnectionId<'_>> for ConnectionId {
    type Error = QuicError;

    fn try_from(value: &quiche::ConnectionId<'_>) -> Result<Self, Self::Error> {
        if value.len() != Self::LEN {
            return Err(QuicheError::InvalidPacket.into());
        }

        let bytes: [u8; 16] = value
            .as_ref()
            .try_into()
            .map_err(|_| QuicError::Quiche(QuicheError::InvalidPacket))?;
        Ok(Self(Uuid::from(bytes)))
    }
}

impl ConnectionId {
    /// Number of bytes in a connection id, determined by [`Uuid`]
    pub const LEN: usize = 16;

    /// Genenerate a random connection id
    pub fn random() -> Self {
        thread_local! {
            static GEN: RefCell<V7Generator<ThreadRng>> = RefCell::new(V7Generator::new(thread_rng()));
        }

        Self(GEN.with(|g| g.borrow_mut().generate()))
    }

    /// As [`quiche::ConnectionId`], borrowed
    pub fn as_cid(&self) -> quiche::ConnectionId<'_> {
        quiche::ConnectionId::from_ref(self.0.as_bytes())
    }

    /// As [`quiche::ConnectionId`], owned ([`Vec`] underlying)
    pub fn as_cid_owned(&self) -> quiche::ConnectionId<'static> {
        quiche::ConnectionId::from_vec(self.0.as_bytes().to_vec())
    }
}

/// A quic connection
#[allow(private_interfaces)]
pub type Connection = Shared<ConnInner>;

impl Connection {
    pub(super) fn spawn(&self, socket: Rc<UdpSocket>) {
        if self.with(|s| s.handle.is_some()) {
            return;
        }

        let handle = compio_runtime::spawn({
            let this = self.clone();
            async move {
                let mut io = Io {
                    buf: vec![0; 1024],
                    socket,
                };
                loop {
                    match this.send_impl(io).await {
                        (Ok(()), ret) => {
                            io = ret;
                        }
                        (Err(e), _) => {
                            return Err(e);
                        }
                    }
                }
            }
        });

        self.with(|s| s.handle = Some(handle));
    }

    async fn send_impl(&self, mut io: Io) -> IoResult<()> {
        if self.with(|s| s.quic.is_draining()) {
            return (Ok(()), io);
        }

        loop {
            // Read packet to send
            let (len, info) = match self.with(|s| s.quic.send(&mut io.buf)) {
                Ok(res) => res,
                Err(QuicheError::Done) => break,
                Err(e) => {
                    return (Err(e.into()), io);
                }
            };

            if info.at > Instant::now() {
                compio_runtime::time::sleep_until(info.at).await;
            }

            (_, io) = try_io!(io.send_all(len, info.to).await);
        }

        (Ok(()), io)
    }
}

pub(crate) struct ConnInner {
    pub(super) quic: quiche::Connection,
    id: ConnectionId,
    wakers: BTreeMap<StreamId, Waker>,
    next_stream_id: StreamId,
    handle: Option<compio_runtime::Task<QuicResult<()>>>,
}

impl ConnInner {
    pub(super) fn accept(
        tuple: FourTuple,
        config: &mut quiche::Config,
    ) -> QuicResult<(ConnectionId, Self)> {
        let id = ConnectionId::random();
        let inner = quiche::accept(&id.as_cid(), None, tuple.local, tuple.peer, config)?;
        let this = Self {
            quic: inner,
            id,
            wakers: Default::default(),
            next_stream_id: StreamId::new_bi(0, true),
            handle: None,
        };

        Ok((id, this))
    }

    pub(super) fn connect(
        server_name: Option<&str>,
        tuple: FourTuple,
        config: &mut quiche::Config,
    ) -> QuicResult<(ConnectionId, Self)> {
        let id = ConnectionId::random();
        let quic = quiche::connect(server_name, &id.as_cid(), tuple.local, tuple.peer, config)?;
        let this = Self {
            quic,
            id,
            wakers: Default::default(),
            next_stream_id: StreamId::new_bi(0, false),
            handle: None,
        };

        Ok((id, this))
    }

    /// If the connection on a server side
    pub fn is_server(&self) -> bool {
        self.quic.is_server()
    }

    /// Get the next stream id
    pub fn id(&self) -> &ConnectionId {
        &self.id
    }
}
