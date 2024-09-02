use compio_quic::ServerBuilder;
use tracing_subscriber::EnvFilter;

#[compio_macros::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let cert = cert.der().clone();
    let key_der = key_pair.serialize_der().try_into().unwrap();

    let endpoint = ServerBuilder::new_with_single_cert(vec![cert], key_der)
        .unwrap()
        .with_key_log()
        .bind("[::1]:4433")
        .await
        .unwrap();

    if let Some(incoming) = endpoint.wait_incoming().await {
        let conn = incoming.await.unwrap();

        let (mut send, mut recv) = conn.accept_bi().await.unwrap();

        let mut buf = vec![];
        recv.read_to_end(&mut buf).await.unwrap();
        println!("{:?}", buf);

        send.write(&[4, 5, 6]).await.unwrap();
        send.finish().unwrap();

        conn.closed().await;
    }

    endpoint.close(0u32.into(), b"");
    endpoint.shutdown().await.unwrap();
}
