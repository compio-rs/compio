cfg_if::cfg_if! {
    if #[cfg(windows)] {
        #[path = "windows.rs"]
        mod sys;
    } else if #[cfg(all(target_os = "linux", target_env = "gnu"))] {
        #[path = "glibc.rs"]
        mod sys;
    } else if #[cfg(unix)] {
        #[path = "unix.rs"]
        mod sys;
    }
}

use std::{
    future::Future,
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6},
};

use compio_buf::{buf_try, BufResult};
use either::Either;

pub async fn resolve_sock_addrs(
    host: &str,
    port: u16,
) -> io::Result<std::vec::IntoIter<SocketAddr>> {
    use std::task::Poll;

    use compio_runtime::event::Event;

    let mut resolver = sys::AsyncResolver::new(host, port)?;
    let mut hints: sys::addrinfo = unsafe { std::mem::zeroed() };
    hints.ai_family = sys::AF_UNSPEC as _;
    hints.ai_socktype = sys::SOCK_STREAM;
    hints.ai_protocol = sys::IPPROTO_TCP;

    let event = Event::new()?;
    let handle = event.handle()?;
    match unsafe { resolver.call(&hints, handle) } {
        Poll::Ready(res) => {
            res?;
        }
        Poll::Pending => {
            event.wait().await?;
        }
    }

    unsafe { resolver.addrs() }
}

#[allow(unused)]
fn to_addrs(mut result: *mut sys::addrinfo, port: u16) -> std::vec::IntoIter<SocketAddr> {
    use socket2::SockAddr;

    let mut addrs = vec![];
    while let Some(info) = unsafe { result.as_ref() } {
        let addr = unsafe {
            SockAddr::try_init(|buffer, len| {
                std::slice::from_raw_parts_mut::<u8>(buffer.cast(), info.ai_addrlen as _)
                    .copy_from_slice(std::slice::from_raw_parts::<u8>(
                        info.ai_addr.cast(),
                        info.ai_addrlen as _,
                    ));
                *len = info.ai_addrlen as _;
                Ok(())
            })
        }
        // it is always Ok
        .unwrap()
        .1;
        if let Some(mut addr) = addr.as_socket() {
            addr.set_port(port);
            addrs.push(addr)
        }
        result = info.ai_next;
    }
    addrs.into_iter()
}

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

macro_rules! impl_to_socket_addrs_async {
    ($($t:ty),* $(,)?) => {
        $(
            impl ToSocketAddrsAsync for $t {
                type Iter = std::iter::Once<SocketAddr>;

                async fn to_socket_addrs_async(&self) -> io::Result<Self::Iter> {
                    Ok(std::iter::once(SocketAddr::from(*self)))
                }
            }
        )*
    }
}

impl_to_socket_addrs_async![
    SocketAddr,
    SocketAddrV4,
    SocketAddrV6,
    (IpAddr, u16),
    (Ipv4Addr, u16),
    (Ipv6Addr, u16),
];

impl ToSocketAddrsAsync for (&str, u16) {
    type Iter = Either<std::iter::Once<SocketAddr>, std::vec::IntoIter<SocketAddr>>;

    async fn to_socket_addrs_async(&self) -> io::Result<Self::Iter> {
        let (host, port) = self;
        if let Ok(addr) = host.parse::<Ipv4Addr>() {
            return Ok(Either::Left(std::iter::once(SocketAddr::from((
                addr, *port,
            )))));
        }
        if let Ok(addr) = host.parse::<Ipv6Addr>() {
            return Ok(Either::Left(std::iter::once(SocketAddr::from((
                addr, *port,
            )))));
        }

        resolve_sock_addrs(host, *port).await.map(Either::Right)
    }
}

impl ToSocketAddrsAsync for (String, u16) {
    type Iter = <(&'static str, u16) as ToSocketAddrsAsync>::Iter;

    async fn to_socket_addrs_async(&self) -> io::Result<Self::Iter> {
        (&*self.0, self.1).to_socket_addrs_async().await
    }
}

impl ToSocketAddrsAsync for str {
    type Iter = <(&'static str, u16) as ToSocketAddrsAsync>::Iter;

    async fn to_socket_addrs_async(&self) -> io::Result<Self::Iter> {
        if let Ok(addr) = self.parse::<SocketAddr>() {
            return Ok(Either::Left(std::iter::once(addr)));
        }

        let (host, port_str) = self
            .rsplit_once(':')
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid socket address"))?;
        let port: u16 = port_str
            .parse()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid port value"))?;
        (host, port).to_socket_addrs_async().await
    }
}

impl ToSocketAddrsAsync for String {
    type Iter = <(&'static str, u16) as ToSocketAddrsAsync>::Iter;

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

pub async fn each_addr<T, F: Future<Output = io::Result<T>>>(
    addr: impl ToSocketAddrsAsync,
    f: impl Fn(SocketAddr) -> F,
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

pub async fn first_addr_buf<T, B, F: Future<Output = BufResult<T, B>>>(
    addr: impl ToSocketAddrsAsync,
    buffer: B,
    f: impl FnOnce(SocketAddr, B) -> F,
) -> BufResult<T, B> {
    let (mut addrs, buffer) = buf_try!(addr.to_socket_addrs_async().await, buffer);
    if let Some(addr) = addrs.next() {
        let (res, buffer) = buf_try!(f(addr, buffer).await);
        BufResult(Ok(res), buffer)
    } else {
        BufResult(
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "could not operate on first address",
            )),
            buffer,
        )
    }
}
