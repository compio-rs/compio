use std::{io, net::SocketAddr};

use futures_util::{
    stream::{iter, once},
    Stream,
};

#[cfg(windows)]
pub fn resolve_sock_addrs(
    host: &str,
    port: u16,
) -> impl Stream<Item = io::Result<SocketAddr>> + '_ {
    use futures_util::TryStreamExt;

    once(async move {
        let op = compio_driver::op::ResolveSockAddrs::new(host, port);
        let (_, op) = compio_buf::buf_try!(@try compio_runtime::submit(op).await);
        io::Result::Ok(iter(op.sock_addrs().into_iter().map(Ok)))
    })
    .try_flatten()
}

#[cfg(unix)]
pub fn resolve_sock_addrs(
    host: &str,
    port: u16,
) -> impl Stream<Item = io::Result<SocketAddr>> + '_ {
    use std::{future::ready, net::ToSocketAddrs};

    use futures_util::future::Either;

    match (host, port).to_socket_addrs() {
        Ok(addrs) => Either::Left(iter(addrs.map(Ok))),
        Err(e) => Either::Right(once(ready(Err(e)))),
    }
}
