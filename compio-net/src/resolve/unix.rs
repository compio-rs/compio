use std::{
    io,
    net::{SocketAddr, ToSocketAddrs},
};

use compio_runtime::ResumeUnwind;

pub async fn resolve_sock_addrs(
    host: &str,
    port: u16,
) -> io::Result<std::vec::IntoIter<SocketAddr>> {
    let host = host.to_string();
    compio_runtime::spawn_blocking(move || (host, port).to_socket_addrs())
        .await
        .resume_unwind()
        .expect("shouldn't be canceled")
}
