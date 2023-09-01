//! Network related.
//!
//! Currently, TCP/UDP/Unix socket are implemented.

mod socket;
pub(crate) use socket::*;

mod tcp;
pub use tcp::*;

mod udp;
pub use udp::*;

use socket2::SockAddr;
use std::{
    future::Future,
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6, ToSocketAddrs},
};

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

macro_rules! impl_raw_fd {
    ($t:ty, $inner:ident) => {
        impl crate::driver::AsRawFd for $t {
            fn as_raw_fd(&self) -> crate::driver::RawFd {
                self.$inner.as_raw_fd()
            }
        }
        impl crate::driver::FromRawFd for $t {
            unsafe fn from_raw_fd(fd: crate::driver::RawFd) -> Self {
                Self {
                    $inner: crate::driver::FromRawFd::from_raw_fd(fd),
                }
            }
        }
        impl crate::driver::IntoRawFd for $t {
            fn into_raw_fd(self) -> crate::driver::RawFd {
                self.$inner.into_raw_fd()
            }
        }
    };
}

pub(crate) use impl_raw_fd;
