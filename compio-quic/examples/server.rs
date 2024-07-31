use compio_quic::Endpoint;
use tracing_subscriber::filter::LevelFilter;

#[compio_macros::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_max_level(LevelFilter::TRACE)
        .init();

    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let cert_chain = vec![cert.cert.into()];
    let key_der = cert.key_pair.serialize_der().try_into().unwrap();

    let endpoint = Endpoint::server()
        .with_single_cert(cert_chain, key_der)
        .unwrap()
        .with_alpn_protocols(&["hq-29"])
        .with_key_log()
        .bind("[::1]:4433")
        .await
        .unwrap();

    if let Some(incoming) = endpoint.wait_incoming().await {
        let conn = incoming.await.unwrap();
        conn.closed().await;
    }

    endpoint.close(0u32.into(), "").await.unwrap();
}
