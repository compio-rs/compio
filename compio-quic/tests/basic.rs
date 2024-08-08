use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::{Duration, Instant},
};

use compio_quic::{ClientBuilder, ConnectionError, Endpoint, TransportConfig};
use futures_util::join;

mod common;
use common::{config_pair, subscribe};

#[compio_macros::test]
#[cfg_attr(target_os = "windows", ignore)] // FIXME: ERROR_PORT_UNREACHABLE
async fn handshake_timeout() {
    let _guard = subscribe();

    let endpoint = Endpoint::client("127.0.0.1:0").await.unwrap();

    const IDLE_TIMEOUT: Duration = Duration::from_millis(100);

    let mut transport_config = TransportConfig::default();
    transport_config
        .max_idle_timeout(Some(IDLE_TIMEOUT.try_into().unwrap()))
        .initial_rtt(Duration::from_millis(10));
    let mut client_config = ClientBuilder::new_with_no_server_verification().build();
    client_config.transport_config(Arc::new(transport_config));

    let start = Instant::now();
    match endpoint
        .connect(
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 1),
            "localhost",
            Some(client_config),
        )
        .unwrap()
        .await
    {
        Err(ConnectionError::TimedOut) => {}
        Err(e) => panic!("unexpected error: {e:?}"),
        Ok(_) => panic!("unexpected success"),
    }
    let dt = start.elapsed();
    assert!(dt > IDLE_TIMEOUT && dt < 2 * IDLE_TIMEOUT);
}

#[compio_macros::test]
async fn close_endpoint() {
    let _guard = subscribe();

    let endpoint = ClientBuilder::new_with_no_server_verification()
        .bind("127.0.0.1:0")
        .await
        .unwrap();

    let conn = endpoint
        .connect(
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 1),
            "localhost",
            None,
        )
        .unwrap();

    compio_runtime::spawn(endpoint.close(0u32.into(), "")).detach();

    match conn.await {
        Err(ConnectionError::LocallyClosed) => (),
        Err(e) => panic!("unexpected error: {e}"),
        Ok(_) => {
            panic!("unexpected success");
        }
    }
}

async fn endpoint() -> Endpoint {
    let (server_config, client_config) = config_pair(None);
    let mut endpoint = Endpoint::server("127.0.0.1:0", server_config)
        .await
        .unwrap();
    endpoint.default_client_config = Some(client_config);
    endpoint
}

#[compio_macros::test]
async fn read_after_close() {
    let _guard = subscribe();

    let endpoint = endpoint().await;

    const MSG: &[u8] = b"goodbye!";

    join!(
        async {
            let conn = endpoint.wait_incoming().await.unwrap().await.unwrap();
            let mut s = conn.open_uni().unwrap();
            s.write_all(MSG).await.unwrap();
            s.finish().unwrap();
            // Wait for the stream to be closed, one way or another.
            let _ = s.stopped().await;
        },
        async {
            let conn = endpoint
                .connect(endpoint.local_addr().unwrap(), "localhost", None)
                .unwrap()
                .await
                .unwrap();
            let mut recv = conn.accept_uni().await.unwrap();
            let mut buf = vec![];
            recv.read_to_end(&mut buf).await.unwrap();
            assert_eq!(buf, MSG);
        },
    );
}

#[compio_macros::test]
async fn export_keying_material() {
    let _guard = subscribe();

    let endpoint = endpoint().await;

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
    let mut buf1 = [0u8; 64];
    let mut buf2 = [0u8; 64];
    conn1
        .export_keying_material(&mut buf1, b"qaq", b"qwq")
        .unwrap();
    conn2
        .export_keying_material(&mut buf2, b"qaq", b"qwq")
        .unwrap();
    assert_eq!(buf1, buf2);
}

#[compio_macros::test]
async fn zero_rtt() {
    let _guard = subscribe();

    let endpoint = endpoint().await;

    const MSG0: &[u8] = b"zero";
    const MSG1: &[u8] = b"one";

    join!(
        async {
            for _ in 0..2 {
                let conn = endpoint
                    .wait_incoming()
                    .await
                    .unwrap()
                    .accept()
                    .unwrap()
                    .into_0rtt()
                    .unwrap();
                join!(
                    async {
                        while let Ok(mut recv) = conn.accept_uni().await {
                            let mut buf = vec![];
                            recv.read_to_end(&mut buf).await.unwrap();
                            assert_eq!(buf, MSG0);
                        }
                    },
                    async {
                        let mut send = conn.open_uni().unwrap();
                        send.write_all(MSG0).await.unwrap();
                        send.finish().unwrap();
                        conn.accepted_0rtt().await.unwrap();
                        let mut send = conn.open_uni().unwrap();
                        send.write_all(MSG1).await.unwrap();
                        send.finish().unwrap();
                        // no need to wait for the stream to be closed due to
                        // the `while` loop above
                    },
                );
            }
        },
        async {
            {
                let conn = endpoint
                    .connect(endpoint.local_addr().unwrap(), "localhost", None)
                    .unwrap()
                    .into_0rtt()
                    .unwrap_err()
                    .await
                    .unwrap();

                let mut buf = vec![];
                let mut recv = conn.accept_uni().await.unwrap();
                recv.read_to_end(&mut buf).await.expect("read_to_end");
                assert_eq!(buf, MSG0);

                buf.clear();
                let mut recv = conn.accept_uni().await.unwrap();
                recv.read_to_end(&mut buf).await.expect("read_to_end");
                assert_eq!(buf, MSG1);
            }

            let conn = endpoint
                .connect(endpoint.local_addr().unwrap(), "localhost", None)
                .unwrap()
                .into_0rtt()
                .unwrap();

            let mut send = conn.open_uni().unwrap();
            send.write_all(MSG0).await.unwrap();
            send.finish().unwrap();

            let mut buf = vec![];
            let mut recv = conn.accept_uni().await.unwrap();
            recv.read_to_end(&mut buf).await.expect("read_to_end");
            assert_eq!(buf, MSG0);

            assert!(conn.accepted_0rtt().await.unwrap());

            buf.clear();
            let mut recv = conn.accept_uni().await.unwrap();
            recv.read_to_end(&mut buf).await.expect("read_to_end");
            assert_eq!(buf, MSG1);
        },
    );
}

#[compio_macros::test]
async fn two_datagram_readers() {
    let _guard = subscribe();

    let endpoint = endpoint().await;

    const MSG1: &[u8] = b"one";
    const MSG2: &[u8] = b"two";

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

    let ev = event_listener::Event::new();

    let (a, b, _) = join!(
        async {
            let x = conn1.recv_datagram().await.unwrap();
            ev.notify(1);
            x
        },
        async {
            let x = conn1.recv_datagram().await.unwrap();
            ev.notify(1);
            x
        },
        async {
            conn2.send_datagram(MSG1.into()).unwrap();
            ev.listen().await;
            conn2.send_datagram_wait(MSG2.into()).await.unwrap();
        }
    );

    assert!(a == MSG1 || b == MSG1);
    assert!(a == MSG2 || b == MSG2);
}
