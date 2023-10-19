use std::{io, net::SocketAddr};

use crate::ToSocketAddrsStream;

#[cfg(not(feature = "generators"))]
pub fn wrap_stream(
    args: impl ToSocketAddrsStream,
) -> impl futures_util::Stream<Item = io::Result<SocketAddr>> {
    async_stream::stream! {
        let s = args.to_socket_addrs_stream();
        for await item in s {
            yield item;
        }
    }
}

#[cfg(feature = "generators")]
type SocketAddrsItem = io::Result<SocketAddr>;

#[cfg(feature = "generators")]
#[stream_future::stream(SocketAddrsItem, lifetime = 'a)]
pub async fn wrap_stream<'a>(args: impl ToSocketAddrsStream + 'a) {
    use futures_util::StreamExt;

    let s = args.to_socket_addrs_stream();
    let mut s = std::pin::pin!(s);
    while let Some(item) = s.next().await {
        yield item;
    }
}
