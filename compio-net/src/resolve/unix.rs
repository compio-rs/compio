use std::{
    io,
    net::{SocketAddr, ToSocketAddrs},
};

pub async fn resolve_sock_addrs(
    host: &str,
    port: u16,
) -> io::Result<std::vec::IntoIter<SocketAddr>> {
    let host = host.to_string();
    compio_runtime::spawn_blocking(move || (host, port).to_socket_addrs()).await
}
