//! Network related.
//!
//! Currently, TCP/UDP/Unix socket are implemented.

#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![warn(missing_docs)]

use std::{
    future::Future,
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6, ToSocketAddrs},
};

use compio_buf::BufResult;
pub(crate) use socket::*;
use socket2::SockAddr;
pub use tcp::*;
pub use udp::*;
pub use unix::*;

#[cfg(target_os = "linux")]
pub mod linux_ext;
mod socket;
mod tcp;
mod udp;
mod unix;

/// A trait for objects which can be converted or resolved to one or more
/// [`SockAddr`] values.
///
/// See [`ToSocketAddrs`].
pub trait ToSockAddrs {
    /// See [`ToSocketAddrs::Iter`].
    type Iter: Iterator<Item = SockAddr>;

    /// See [`ToSocketAddrs::to_socket_addrs`].
    fn to_sock_addrs(&self) -> io::Result<Self::Iter>;
}

// impl_to_sock_addrs_for_into_socket_addr
macro_rules! itsafisa {
    ($t:ty) => {
        impl ToSockAddrs for $t {
            type Iter =
                std::iter::Map<<$t as std::net::ToSocketAddrs>::Iter, fn(SocketAddr) -> SockAddr>;

            fn to_sock_addrs(&self) -> io::Result<Self::Iter> {
                std::net::ToSocketAddrs::to_socket_addrs(self)
                    .map(|iter| iter.map(SockAddr::from as _))
            }
        }
    };
}

itsafisa!(SocketAddr);
itsafisa!(SocketAddrV4);
itsafisa!(SocketAddrV6);
itsafisa!(str);
itsafisa!(String);
itsafisa!((IpAddr, u16));
itsafisa!((Ipv4Addr, u16));
itsafisa!((Ipv6Addr, u16));
itsafisa!((String, u16));

impl ToSockAddrs for (&str, u16) {
    type Iter = std::iter::Map<std::vec::IntoIter<SocketAddr>, fn(SocketAddr) -> SockAddr>;

    fn to_sock_addrs(&self) -> io::Result<Self::Iter> {
        ToSocketAddrs::to_socket_addrs(self).map(|iter| iter.map(SockAddr::from as _))
    }
}

impl ToSockAddrs for SockAddr {
    type Iter = std::option::IntoIter<SockAddr>;

    fn to_sock_addrs(&self) -> io::Result<Self::Iter> {
        Ok(Some(self.clone()).into_iter())
    }
}

impl<'a> ToSockAddrs for &'a [SockAddr] {
    type Iter = std::iter::Cloned<std::slice::Iter<'a, SockAddr>>;

    fn to_sock_addrs(&self) -> io::Result<Self::Iter> {
        Ok(self.iter().cloned())
    }
}

impl<T: ToSockAddrs + ?Sized> ToSockAddrs for &T {
    type Iter = T::Iter;

    fn to_sock_addrs(&self) -> io::Result<Self::Iter> {
        (**self).to_sock_addrs()
    }
}

fn each_addr<T>(
    addr: impl ToSockAddrs,
    mut f: impl FnMut(SockAddr) -> io::Result<T>,
) -> io::Result<T> {
    let addrs = addr.to_sock_addrs()?;
    let mut last_err = None;
    for addr in addrs {
        match f(addr) {
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

#[allow(dead_code)]
async fn each_addr_async<T, F: Future<Output = io::Result<T>>>(
    addr: impl ToSockAddrs,
    mut f: impl FnMut(SockAddr) -> F,
) -> io::Result<T> {
    let addrs = addr.to_sock_addrs()?;
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

#[allow(dead_code)]
async fn each_addr_async_buf<T, B, F: Future<Output = BufResult<T, B>>>(
    addr: impl ToSockAddrs,
    mut buffer: B,
    mut f: impl FnMut(SockAddr, B) -> F,
) -> BufResult<T, B> {
    match addr.to_sock_addrs() {
        Ok(addrs) => {
            let mut last_err = None;
            let mut res;
            for addr in addrs {
                BufResult(res, buffer) = f(addr, buffer).await;
                match res {
                    Ok(l) => return BufResult(Ok(l), buffer),
                    Err(e) => last_err = Some(e),
                }
            }
            BufResult(
                Err(last_err.unwrap_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "could not resolve to any addresses",
                    )
                })),
                buffer,
            )
        }
        Err(e) => BufResult(Err(e), buffer),
    }
}
