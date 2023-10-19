//! Network related.
//!
//! Currently, TCP/UDP/Unix socket are implemented.

#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![cfg_attr(feature = "generators", feature(generators))]
#![warn(missing_docs)]

mod resolve;
mod socket;
mod tcp;
mod udp;
mod unix;

use std::{
    future::Future,
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6},
};

pub(crate) use socket::*;
pub use tcp::*;
pub use udp::*;
pub use unix::*;

/// A trait for objects which can be converted or resolved to one or more
/// [`SocketAddr`] values.
///
/// See [`std::net::ToSocketAddrs`].
#[allow(async_fn_in_trait)]
pub trait ToSocketAddrsAsync {
    /// See [`std::net::ToSocketAddrs::Iter`].
    type Iter: Iterator<Item = SocketAddr>;

    /// See [`std::net::ToSocketAddrs::to_socket_addrs`].
    async fn to_socket_addrs_async(&self) -> io::Result<Self::Iter>;
}

// impl_to_sock_addrs_for_into_socket_addr
macro_rules! itsafisa {
    ($t:ty) => {
        impl ToSocketAddrsAsync for $t {
            type Iter = std::option::IntoIter<SocketAddr>;

            async fn to_socket_addrs_async(&self) -> io::Result<Self::Iter> {
                Ok(Some(SocketAddr::from(*self)).into_iter())
            }
        }
    };
}

itsafisa!(SocketAddr);
itsafisa!(SocketAddrV4);
itsafisa!(SocketAddrV6);
itsafisa!((IpAddr, u16));
itsafisa!((Ipv4Addr, u16));
itsafisa!((Ipv6Addr, u16));

impl ToSocketAddrsAsync for (&str, u16) {
    type Iter = std::vec::IntoIter<SocketAddr>;

    async fn to_socket_addrs_async(&self) -> io::Result<Self::Iter> {
        let (host, port) = self;
        if let Ok(addr) = host.parse::<Ipv4Addr>() {
            return Ok(vec![SocketAddr::from((addr, *port))].into_iter());
        }
        if let Ok(addr) = host.parse::<Ipv6Addr>() {
            return Ok(vec![SocketAddr::from((addr, *port))].into_iter());
        }

        resolve::resolve_sock_addrs(host, *port).await
    }
}

impl ToSocketAddrsAsync for (String, u16) {
    type Iter = std::vec::IntoIter<SocketAddr>;

    async fn to_socket_addrs_async(&self) -> io::Result<Self::Iter> {
        (&*self.0, self.1).to_socket_addrs_async().await
    }
}

impl ToSocketAddrsAsync for str {
    type Iter = std::vec::IntoIter<SocketAddr>;

    async fn to_socket_addrs_async(&self) -> io::Result<Self::Iter> {
        if let Ok(addr) = self.parse::<SocketAddr>() {
            return Ok(vec![addr].into_iter());
        }

        let (host, port_str) = self.rsplit_once(':').expect("invalid socket address");
        let port: u16 = port_str.parse().expect("invalid port value");
        (host, port).to_socket_addrs_async().await
    }
}

impl ToSocketAddrsAsync for String {
    type Iter = std::vec::IntoIter<SocketAddr>;

    async fn to_socket_addrs_async(&self) -> io::Result<Self::Iter> {
        self.as_str().to_socket_addrs_async().await
    }
}

impl<'a> ToSocketAddrsAsync for &'a [SocketAddr] {
    type Iter = std::iter::Copied<std::slice::Iter<'a, SocketAddr>>;

    async fn to_socket_addrs_async(&self) -> io::Result<Self::Iter> {
        Ok(self.iter().copied())
    }
}

impl<T: ToSocketAddrsAsync + ?Sized> ToSocketAddrsAsync for &T {
    type Iter = T::Iter;

    async fn to_socket_addrs_async(&self) -> io::Result<Self::Iter> {
        (**self).to_socket_addrs_async().await
    }
}

#[cfg(feature = "runtime")]
async fn each_addr<T, F: Future<Output = io::Result<T>>>(
    addr: impl ToSocketAddrsAsync,
    mut f: impl FnMut(SocketAddr) -> F,
) -> io::Result<T> {
    let addrs = addr.to_socket_addrs_async().await?;
    let mut last_err = None;
    for addr in addrs {
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
