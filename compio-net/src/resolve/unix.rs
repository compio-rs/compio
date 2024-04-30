use std::{
    io,
    net::{SocketAddr, ToSocketAddrs},
    panic::resume_unwind,
};

pub async fn resolve_sock_addrs(
    host: &str,
    port: u16,
) -> io::Result<std::vec::IntoIter<SocketAddr>> {
    let host = host.to_string();
    compio_runtime::spawn_blocking(move || (host, port).to_socket_addrs())
        .await
        .unwrap_or_else(|e| resume_unwind(e))
}
