use std::net::{IpAddr, Ipv6Addr, SocketAddr};

use compio_quic::ClientBuilder;
use tracing_subscriber::EnvFilter;

#[compio_macros::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let endpoint = ClientBuilder::new_with_no_server_verification()
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

        let (mut send, mut recv) = conn.open_bi().unwrap();
        send.write(&[1, 2, 3]).await.unwrap();
        send.finish().unwrap();

        let mut buf = vec![];
        recv.read_to_end(&mut buf).await.unwrap();
        println!("{:?}", buf);

        conn.close(1u32.into(), b"bye");
    }

    endpoint.shutdown().await.unwrap();
}
