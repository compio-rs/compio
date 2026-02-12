mod config;
mod protocol;
mod resolver;

use std::{io, net::SocketAddr};

pub use resolver::AsyncResolver;

pub async fn resolve_sock_addrs(
    host: &str,
    port: u16,
) -> io::Result<std::vec::IntoIter<SocketAddr>> {
    let resolver = AsyncResolver::new()?;
    Ok(resolver
        .lookup(host)
        .await?
        .map(|mut addr| {
            addr.set_port(port);
            addr
        })
        .collect::<Vec<_>>()
        .into_iter())
}
