cfg_select! {
    windows => {
        #[path = "windows.rs"]
        mod sys;
    }
    unix => {
        #[path = "unix.rs"]
        mod sys;
    }
    _ => {}
}

use std::{
    future::{Future, Ready, ready},
    io,
    iter::from_fn,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6},
    pin::pin,
    time::Duration,
};

use compio_buf::{BufResult, buf_try};
use compio_runtime::time::sleep;
use either::Either;
use futures_util::{
    StreamExt,
    future::Either as EitherFut,
    stream::{self, FuturesUnordered},
};
pub use sys::resolve_sock_addrs;

/// A trait for objects which can be converted or resolved to one or more
/// [`SocketAddr`] values.
///
/// See [`std::net::ToSocketAddrs`].
///
/// # Cancel safety
///
/// All implementation of [`ToSocketAddrsAsync`] in this crate is safe to cancel
/// by dropping the future. The Glibc impl may leak the control block if the
/// task is not completed when dropping.
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

static HAPPY_EYEBALLS_STAGGER_DELAY: Duration = Duration::from_millis(200);

fn sort_addrs(addrs: impl IntoIterator<Item = SocketAddr>) -> impl Iterator<Item = SocketAddr> {
    let (v6, v4): (Vec<_>, Vec<_>) = addrs.into_iter().partition(SocketAddr::is_ipv6);
    let (mut iter6, mut iter4) = (v6.into_iter(), v4.into_iter());
    let mut next_is_v6 = true;

    from_fn(move || {
        let res = if next_is_v6 {
            iter6.next().or_else(|| iter4.next())
        } else {
            iter4.next().or_else(|| iter6.next())
        };
        next_is_v6 = !next_is_v6;
        res
    })
}

pub async fn each_addr<T>(
    addr: impl ToSocketAddrsAsync,
    racing: bool,
    f: impl AsyncFn(SocketAddr) -> io::Result<T>,
) -> io::Result<T> {
    let addrs = addr.to_socket_addrs_async().await?;

    let futs = if racing {
        let futs: FuturesUnordered<_> = sort_addrs(addrs)
            .into_iter()
            .enumerate()
            .map(async |(i, addr)| {
                if i != 0 {
                    sleep(HAPPY_EYEBALLS_STAGGER_DELAY * i as u32).await
                }
                f(addr).await
            })
            .collect();
        EitherFut::Left(futs)
    } else {
        EitherFut::Right(stream::iter(addrs).then(async |addr| f(addr).await))
    };
    let mut futs = pin!(futs);

    let mut last_err = None;
    while let Some(res) = futs.next().await {
        match res {
            Ok(conn) => return Ok(conn),
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

pub async fn first_addr_buf<T, B>(
    addr: impl ToSocketAddrsAsync,
    buffer: B,
    f: impl AsyncFnOnce(SocketAddr, B) -> BufResult<T, B>,
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

pub async fn first_addr_buf_zerocopy<B, F>(
    addr: impl ToSocketAddrsAsync,
    buffer: B,
    f: impl AsyncFnOnce(SocketAddr, B) -> BufResult<usize, F>,
) -> BufResult<usize, Either<Ready<B>, F>>
where
    F: Future<Output = B>,
{
    fn ret<T, F>(fut: T) -> Either<Ready<T>, F> {
        Either::Left(ready(fut))
    }

    let mut addrs = match addr.to_socket_addrs_async().await {
        Ok(addrs) => addrs,
        Err(e) => return BufResult(Err(e), ret(buffer)),
    };
    if let Some(addr) = addrs.next() {
        let BufResult(res, fut) = f(addr, buffer).await;
        BufResult(res, Either::Right(fut))
    } else {
        BufResult(
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "could not operate on first address",
            )),
            ret(buffer),
        )
    }
}
