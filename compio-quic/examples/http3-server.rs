use compio_buf::bytes::Bytes;
use compio_quic::ServerBuilder;
use http::{HeaderMap, Response};
use tracing_subscriber::EnvFilter;

#[compio_macros::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let rcgen::CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let cert = cert.der().clone();
    let key_der = signing_key.serialize_der().try_into().unwrap();

    let endpoint = ServerBuilder::new_with_single_cert(vec![cert], key_der)
        .unwrap()
        .with_key_log()
        .with_alpn_protocols(&["h3"])
        .bind("[::1]:4433")
        .await
        .unwrap();

    while let Some(incoming) = endpoint.wait_incoming().await {
        compio_runtime::spawn(async move {
            let conn = incoming.await.unwrap();
            println!("Accepted connection from {}", conn.remote_address());

            let mut conn = compio_quic::h3::server::builder()
                .build::<_, Bytes>(conn)
                .await
                .unwrap();

            while let Ok(Some(resolver)) = conn.accept().await {
                let (req, mut stream) = resolver.resolve_request().await.unwrap();
                println!("Received request: {req:?}");
                stream
                    .send_response(
                        Response::builder()
                            .header("server", "compio-quic")
                            .body(())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                stream
                    .send_data("hello from compio-quic".into())
                    .await
                    .unwrap();
                let mut headers = HeaderMap::new();
                headers.insert("msg", "byebye".parse().unwrap());
                stream.send_trailers(headers).await.unwrap();
            }
        })
        .detach();
    }
}
