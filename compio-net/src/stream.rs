use std::{io, net::SocketAddr};

use async_stream::stream;
use futures_util::Stream;

use crate::ToSocketAddrsStream;

pub fn wrap_stream(args: impl ToSocketAddrsStream) -> impl Stream<Item = io::Result<SocketAddr>> {
    stream! {
        let s = args.to_socket_addrs_stream();
        for await item in s {
            yield item;
        }
    }
}
