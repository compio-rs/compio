use compio_quic::{ConnectionError, Endpoint, TransportConfig};
use synchrony::unsync::async_flag::AsyncFlag;

mod common;
use common::{config_pair, subscribe};
use futures_util::{FutureExt, join};

#[compio_macros::test]
async fn ip_blocking() {
    let _guard = subscribe();

    let (server_config, client_config) = config_pair(None);

    let server = Endpoint::server("127.0.0.1:0", server_config)
        .await
        .unwrap();
    let server_addr = server.local_addr().unwrap();

    let client1 = Endpoint::client("127.0.0.1:0").await.unwrap();
    let client1_addr = client1.local_addr().unwrap();
    let client2 = Endpoint::client("127.0.0.1:0").await.unwrap();

    let shutdown_flag = AsyncFlag::new();
    let shutdown_handle = shutdown_flag.handle();

    let srv = compio_runtime::spawn(async move {
        let wait_fut = shutdown_flag.wait().fuse();
        let mut wait_fut = std::pin::pin!(wait_fut);
        loop {
            let incoming = futures_util::select! {
                 incoming = server.wait_incoming().fuse() => incoming.unwrap(),
                 _ = wait_fut.as_mut() => break,
            };
            if incoming.remote_address() == client1_addr {
                incoming.refuse();
            } else if incoming.remote_address_validated() {
                incoming.await.unwrap();
            } else {
                incoming.retry().unwrap();
            }
        }
        server.shutdown().await.unwrap();
    });

    let e = client1
        .connect(server_addr, "localhost", Some(client_config.clone()))
        .unwrap()
        .await
        .unwrap_err();
    assert!(matches!(e, ConnectionError::ConnectionClosed(_)));
    client2
        .connect(server_addr, "localhost", Some(client_config))
        .unwrap()
        .await
        .unwrap();

    shutdown_handle.notify();

    srv.await.unwrap();

    client2.shutdown().await.unwrap();
    client1.shutdown().await.unwrap();
}

#[compio_macros::test]
async fn stream_id_flow_control() {
    let _guard = subscribe();

    let mut cfg = TransportConfig::default();
    cfg.max_concurrent_uni_streams(1u32.into());

    let (server_config, client_config) = config_pair(Some(cfg));
    let mut endpoint = Endpoint::server("127.0.0.1:0", server_config)
        .await
        .unwrap();
    endpoint.default_client_config = Some(client_config);

    let (conn1, conn2) = join!(
        async {
            endpoint
                .connect(endpoint.local_addr().unwrap(), "localhost", None)
                .unwrap()
                .await
                .unwrap()
        },
        async { endpoint.wait_incoming().await.unwrap().await.unwrap() },
    );

    // If `open_uni_wait` doesn't get unblocked when the previous stream is dropped,
    // this will time out.
    join!(
        async {
            conn1.open_uni_wait().await.unwrap();
        },
        async {
            conn1.open_uni_wait().await.unwrap();
        },
        async {
            conn1.open_uni_wait().await.unwrap();
        },
        async {
            conn2.accept_uni().await.unwrap();
            conn2.accept_uni().await.unwrap();
        }
    );

    drop(conn1);
    drop(conn2);

    endpoint.shutdown().await.unwrap();
}
