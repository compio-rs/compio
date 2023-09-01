mod socket;
pub(crate) use socket::*;

mod tcp;
pub use tcp::*;

use socket2::SockAddr;
use std::{future::Future, io};

pub trait ToSockAddrs {
    type Iter: Iterator<Item = SockAddr>;

    fn to_sock_addrs(&self) -> io::Result<Self::Iter>;
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
