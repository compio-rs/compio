use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    path::PathBuf,
    str::FromStr,
};

use compio_buf::bytes::Buf;
use compio_io::AsyncWriteAtExt;
use compio_net::ToSocketAddrsAsync;
use compio_quic::ClientBuilder;
use http::{Request, Uri};
use tracing_subscriber::EnvFilter;

#[compio_macros::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let args = std::env::args().collect::<Vec<_>>();
    if args.len() != 3 {
        eprintln!("Usage: {} <URI> <OUT>", args[0]);
        std::process::exit(1);
    }

    let uri = Uri::from_str(&args[1]).unwrap();
    let outpath = PathBuf::from(&args[2]);

    let host = uri.host().unwrap();
    let remote = (host, uri.port_u16().unwrap_or(443))
        .to_socket_addrs_async()
        .await
        .unwrap()
        .next()
        .unwrap();

    let endpoint = ClientBuilder::new_with_no_server_verification()
        .with_key_log()
        .with_alpn_protocols(&["h3"])
        .bind(SocketAddr::new(
            if remote.is_ipv6() {
                IpAddr::V6(Ipv6Addr::UNSPECIFIED)
            } else {
                IpAddr::V4(Ipv4Addr::UNSPECIFIED)
            },
            0,
        ))
        .await
        .unwrap();

    {
        println!("Connecting to {host} at {remote}");
        let conn = endpoint.connect(remote, host, None).unwrap().await.unwrap();

        let (mut conn, mut send_req) = compio_quic::h3::client::new(conn).await.unwrap();
        let handle = compio_runtime::spawn(async move { conn.wait_idle().await });

        let req = Request::get(uri).body(()).unwrap();
        let mut stream = send_req.send_request(req).await.unwrap();
        stream.finish().await.unwrap();

        let resp = stream.recv_response().await.unwrap();
        println!("{resp:?}");

        let mut out = compio_fs::File::create(outpath).await.unwrap();
        let mut pos = 0;
        while let Some(mut chunk) = stream.recv_data().await.unwrap() {
            let len = chunk.remaining();
            out.write_all_at(chunk.copy_to_bytes(len), pos)
                .await
                .unwrap();
            pos += len as u64;
        }
        if let Some(headers) = stream.recv_trailers().await.unwrap() {
            println!("{headers:?}");
        }

        drop(send_req);

        handle.await.unwrap();
    }

    endpoint.shutdown().await.unwrap();
}
