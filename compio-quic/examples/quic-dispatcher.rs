use std::num::NonZeroUsize;

use compio_dispatcher::Dispatcher;
use compio_quic::{ClientBuilder, Endpoint, ServerBuilder};
use compio_runtime::spawn;
use futures_util::{stream::FuturesUnordered, StreamExt};

#[compio_macros::main]
async fn main() {
    const THREAD_NUM: usize = 5;
    const CLIENT_NUM: usize = 10;

    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let cert = cert.der().clone();
    let key_der = key_pair.serialize_der().try_into().unwrap();

    let server_config = ServerBuilder::new_with_single_cert(vec![cert.clone()], key_der)
        .unwrap()
        .build();
    let client_config = ClientBuilder::new_with_empty_roots()
        .with_custom_certificate(cert)
        .unwrap()
        .with_no_crls()
        .build();
    let mut endpoint = Endpoint::server("127.0.0.1:0", server_config)
        .await
        .unwrap();
    endpoint.default_client_config = Some(client_config);

    spawn({
        let endpoint = endpoint.clone();
        async move {
            let mut futures = FuturesUnordered::from_iter((0..CLIENT_NUM).map(|i| {
                let endpoint = &endpoint;
                async move {
                    let conn = endpoint
                        .connect(endpoint.local_addr().unwrap(), "localhost", None)
                        .unwrap()
                        .await
                        .unwrap();
                    let mut send = conn.open_uni().unwrap();
                    send.write_all(format!("Hello world {}!", i).as_bytes())
                        .await
                        .unwrap();
                    send.finish().unwrap();
                    send.stopped().await.unwrap();
                }
            }));
            while let Some(()) = futures.next().await {}
        }
    })
    .detach();

    let dispatcher = Dispatcher::builder()
        .worker_threads(NonZeroUsize::new(THREAD_NUM).unwrap())
        .build()
        .unwrap();
    let mut handles = FuturesUnordered::new();
    for _i in 0..CLIENT_NUM {
        let incoming = endpoint.wait_incoming().await.unwrap();
        let handle = dispatcher
            .dispatch(move || async move {
                let conn = incoming.await.unwrap();
                let mut recv = conn.accept_uni().await.unwrap();
                let mut buf = vec![];
                recv.read_to_end(&mut buf).await.unwrap();
                println!("{}", std::str::from_utf8(&buf).unwrap());
            })
            .unwrap();
        handles.push(handle);
    }
    while handles.next().await.is_some() {}
    dispatcher.join().await.unwrap();
}
