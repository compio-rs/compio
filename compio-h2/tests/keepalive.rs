//! Keepalive tests.

use std::time::Duration;

use compio_h2::{ClientBuilder, ServerBuilder};

mod common;

#[compio_macros::test]
async fn keepalive_connection_survives_idle() {
    let timeout = compio_runtime::time::timeout(Duration::from_secs(5), async {
        let cb = ClientBuilder::new()
            .keepalive_interval(Duration::from_millis(100))
            .keepalive_timeout(Duration::from_millis(500));
        let sb = ServerBuilder::new();

        let (mut client, mut server) = common::setup_with_builders(cb, sb).await;

        // Idle for 300ms (keepalive should fire at 100ms)
        compio_runtime::time::sleep(Duration::from_millis(300)).await;

        // Request should still work
        let (resp_fut, _) = client
            .send_request(common::get_request("/alive"), true)
            .await
            .unwrap();
        let (_req, mut send_resp) = server.accept().await.unwrap().unwrap();
        send_resp
            .send_response(common::ok_response(), true)
            .await
            .unwrap();
        let resp = resp_fut.await_response().await.unwrap();
        assert_eq!(resp.status(), 200);
    });
    timeout
        .await
        .expect("keepalive_connection_survives_idle timed out");
}

#[compio_macros::test]
async fn no_keepalive_default() {
    let timeout = compio_runtime::time::timeout(Duration::from_secs(5), async {
        let (mut client, mut server) = common::setup().await;

        // Idle for 200ms with default (no keepalive)
        compio_runtime::time::sleep(Duration::from_millis(200)).await;

        // Request should still work — no keepalive doesn't mean timeout
        let (resp_fut, _) = client
            .send_request(common::get_request("/no-keepalive"), true)
            .await
            .unwrap();
        let (_req, mut send_resp) = server.accept().await.unwrap().unwrap();
        send_resp
            .send_response(common::ok_response(), true)
            .await
            .unwrap();
        let resp = resp_fut.await_response().await.unwrap();
        assert_eq!(resp.status(), 200);
    });
    timeout.await.expect("no_keepalive_default timed out");
}
