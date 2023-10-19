//! Network related.
//!
//! Currently, TCP/UDP/Unix socket are implemented.

#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![warn(missing_docs)]

mod resolve;
mod socket;
mod stream;
mod tcp;
mod udp;
mod unix;

use std::{
    future::{ready, Future},
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6},
};

use futures_util::{future::Either, Stream, StreamExt};
pub(crate) use socket::*;
pub use tcp::*;
pub use udp::*;
pub use unix::*;

/// A trait for objects which can be converted or resolved to one or more
/// [`SockAddr`] values.
///
/// See [`ToSocketAddrs`].
pub trait ToSocketAddrsStream {
    /// See [`ToSocketAddrs::to_socket_addrs`].
    fn to_socket_addrs_stream(&self) -> impl Stream<Item = io::Result<SocketAddr>>;
}

// impl_to_sock_addrs_for_into_socket_addr
macro_rules! itsafisa {
    ($t:ty) => {
        impl ToSocketAddrsStream for $t {
            fn to_socket_addrs_stream(&self) -> impl Stream<Item = io::Result<SocketAddr>> {
                futures_util::stream::once(ready(Ok(SocketAddr::from(*self))))
            }
        }
    };
}

itsafisa!(SocketAddrV4);
itsafisa!(SocketAddrV6);
itsafisa!((IpAddr, u16));
itsafisa!((Ipv4Addr, u16));
itsafisa!((Ipv6Addr, u16));

impl ToSocketAddrsStream for (&str, u16) {
    fn to_socket_addrs_stream(&self) -> impl Stream<Item = io::Result<SocketAddr>> {
        let (host, port) = self;
        if let Ok(addr) = host.parse::<Ipv4Addr>() {
            return Either::Left(Either::Left(futures_util::stream::once(ready(Ok(
                SocketAddr::from((addr, *port)),
            )))));
        }
        if let Ok(addr) = host.parse::<Ipv6Addr>() {
            return Either::Left(Either::Right(futures_util::stream::once(ready(Ok(
                SocketAddr::from((addr, *port)),
            )))));
        }

        Either::Right(resolve::resolve_sock_addrs(host, *port))
    }
}

impl ToSocketAddrsStream for (String, u16) {
    fn to_socket_addrs_stream(&self) -> impl Stream<Item = io::Result<SocketAddr>> {
        let (host, port) = self;
        stream::wrap_stream((host.as_str(), *port))
    }
}

impl ToSocketAddrsStream for str {
    fn to_socket_addrs_stream(&self) -> impl Stream<Item = io::Result<SocketAddr>> {
        if let Ok(addr) = self.parse::<SocketAddr>() {
            Either::Left(stream::wrap_stream(addr))
        } else {
            let (host, port_str) = self.rsplit_once(':').expect("invalid socket address");
            let port: u16 = port_str.parse().expect("invalid port value");
            Either::Right(stream::wrap_stream((host, port)))
        }
    }
}

impl ToSocketAddrsStream for String {
    fn to_socket_addrs_stream(&self) -> impl Stream<Item = io::Result<SocketAddr>> {
        self.as_str().to_socket_addrs_stream()
    }
}

impl ToSocketAddrsStream for SocketAddr {
    fn to_socket_addrs_stream(&self) -> impl Stream<Item = io::Result<SocketAddr>> {
        futures_util::stream::once(ready(Ok(*self)))
    }
}

impl ToSocketAddrsStream for [SocketAddr] {
    fn to_socket_addrs_stream(&self) -> impl Stream<Item = io::Result<SocketAddr>> {
        futures_util::stream::iter(self.iter().map(|addr| Ok(*addr)))
    }
}

impl<T: ToSocketAddrsStream + ?Sized> ToSocketAddrsStream for &T {
    fn to_socket_addrs_stream(&self) -> impl Stream<Item = io::Result<SocketAddr>> {
        (**self).to_socket_addrs_stream()
    }
}

#[cfg(feature = "runtime")]
async fn each_addr<T, F: Future<Output = io::Result<T>>>(
    addr: impl ToSocketAddrsStream,
    mut f: impl FnMut(SocketAddr) -> F,
) -> io::Result<T> {
    let addrs = addr.to_socket_addrs_stream();
    let mut addrs = std::pin::pin!(addrs);
    let mut last_err = None;
    while let Some(addr) = addrs.next().await {
        let addr = addr?;
        match f(addr).await {
            Ok(l) => return Ok(l),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "could not resolve to any addresses",
        )
    }))
}
