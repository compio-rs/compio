use std::{
    io,
    net::{SocketAddr, ToSocketAddrs},
};

use compio_runtime::{ResumeUnwind, SpawnMeta};

pub async fn resolve_sock_addrs(
    host: &str,
    port: u16,
) -> io::Result<std::vec::IntoIter<SocketAddr>> {
    let host = host.to_string();
    // Name the task: its location points here rather than into the code that
    // asked for the address, since this is an `async fn`.
    let meta = SpawnMeta::capture().named("resolve");
    compio_runtime::spawn_blocking_at(move || (host, port).to_socket_addrs(), meta)
        .await
        .resume_unwind()
        .expect("shouldn't be cancelled")
}
