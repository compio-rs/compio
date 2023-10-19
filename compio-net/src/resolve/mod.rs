#[cfg(all(target_os = "linux", target_env = "gnu"))]
mod glibc;

use std::{io, net::SocketAddr};

#[cfg(windows)]
pub async fn resolve_sock_addrs(
    host: &str,
    port: u16,
) -> io::Result<std::vec::IntoIter<SocketAddr>> {
    let op = compio_driver::op::ResolveSockAddrs::new(host, port);
    let (_, op) = compio_buf::buf_try!(@try compio_runtime::submit(op).await);
    op.sock_addrs().map(|vec| vec.into_iter())
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub async fn resolve_sock_addrs(
    host: &str,
    port: u16,
) -> io::Result<std::vec::IntoIter<SocketAddr>> {
    glibc::resolve(host, port).await.map(|vec| vec.into_iter())
}

#[cfg(all(unix, not(all(target_os = "linux", target_env = "gnu"))))]
pub async fn resolve_sock_addrs(
    host: &str,
    port: u16,
) -> io::Result<std::vec::IntoIter<SocketAddr>> {
    use std::net::ToSocketAddrs;

    (host, port).to_socket_addrs()
}
