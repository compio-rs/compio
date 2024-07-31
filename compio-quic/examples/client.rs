use std::net::{IpAddr, Ipv6Addr, SocketAddr};

use compio_quic::Endpoint;
use tracing_subscriber::filter::LevelFilter;

#[compio_macros::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_max_level(LevelFilter::TRACE)
        .init();

    let endpoint = Endpoint::client()
        .with_no_server_verification()
        .with_alpn_protocols(&["hq-29"])
        .with_key_log()
        .bind("[::1]:0")
        .await
        .unwrap();

    {
        let conn = endpoint
            .connect(
                SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 4433),
                "localhost",
                None,
            )
            .unwrap()
            .await
            .unwrap();
        conn.close(1u32.into(), "bye");
        conn.closed().await;
    }
    endpoint.close(0u32.into(), "").await.unwrap();
}
