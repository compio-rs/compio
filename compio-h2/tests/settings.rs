//! Builder settings tests.

use std::time::Duration;

use compio_h2::{ClientBuilder, ServerBuilder};

mod common;

#[compio_macros::test]
async fn client_builder_all_settings() {
    // Smoke test: all client builder settings are accepted without error.
    // Individual settings are validated by dedicated tests (flow control, etc.).
    let cb = ClientBuilder::new()
        .initial_window_size(1 << 20)
        .max_concurrent_streams(100)
        .max_frame_size(32768)
        .max_header_list_size(65536)
        .header_table_size(8192)
        .enable_push(false);
    let sb = ServerBuilder::new();

    let (mut client, mut server) = common::setup_with_builders(cb, sb).await;

    let (resp_fut, _) = client
        .send_request(common::get_request("/builder"), true)
        .await
        .unwrap();
    let (_req, mut send_resp) = server.accept().await.unwrap().unwrap();
    send_resp
        .send_response(common::ok_response(), true)
        .await
        .unwrap();
    let resp = resp_fut.await_response().await.unwrap();
    assert_eq!(resp.status(), 200);
}

#[compio_macros::test]
async fn server_builder_all_settings() {
    // Smoke test: all server builder settings are accepted without error.
    // Individual settings are validated by dedicated tests (flow control, etc.).
    let cb = ClientBuilder::new();
    let sb = ServerBuilder::new()
        .initial_window_size(1 << 20)
        .max_concurrent_streams(100)
        .max_frame_size(32768)
        .max_header_list_size(65536)
        .header_table_size(8192)
        .enable_push(false);

    let (mut client, mut server) = common::setup_with_builders(cb, sb).await;

    let (resp_fut, _) = client
        .send_request(common::get_request("/srv-builder"), true)
        .await
        .unwrap();
    let (_req, mut send_resp) = server.accept().await.unwrap().unwrap();
    send_resp
        .send_response(common::ok_response(), true)
        .await
        .unwrap();
    let resp = resp_fut.await_response().await.unwrap();
    assert_eq!(resp.status(), 200);
}

#[compio_macros::test]
async fn mismatched_frame_sizes() {
    let timeout = compio_runtime::time::timeout(Duration::from_secs(5), async {
        let cb = ClientBuilder::new().max_frame_size(16384);
        let sb = ServerBuilder::new().max_frame_size(32768);

        let (mut client, mut server) = common::setup_with_builders(cb, sb).await;

        // Send body that spans multiple frames at client's 16KB frame limit
        let payload = vec![0x55u8; 48 * 1024];

        let (resp_fut, send_stream) = client
            .send_request(common::post_request("/mismatch"), false)
            .await
            .unwrap();
        let mut ss = send_stream.unwrap();
        common::send_chunked(&mut ss, &payload, 8192).await;

        let (req, mut send_resp) = server.accept().await.unwrap().unwrap();
        let (_, mut recv) = req.into_parts();
        let body = common::recv_all(&mut recv).await;
        assert_eq!(body.len(), 48 * 1024);

        send_resp
            .send_response(common::ok_response(), true)
            .await
            .unwrap();

        let resp = resp_fut.await_response().await.unwrap();
        assert_eq!(resp.status(), 200);
    });
    timeout.await.expect("mismatched_frame_sizes timed out");
}

#[compio_macros::test]
async fn security_asymmetric_max_header_list_size() {
    let timeout = compio_runtime::time::timeout(Duration::from_secs(5), async {
        // Client advertises a 1024-byte header list size limit
        let cb = ClientBuilder::new().max_header_list_size(1024);
        let sb = ServerBuilder::new();

        let (mut client, mut server) = common::setup_with_builders(cb, sb).await;

        let (resp_fut, _) = client
            .send_request(common::get_request("/header-limit"), true)
            .await
            .unwrap();

        let (_req, mut send_resp) = server.accept().await.unwrap().unwrap();

        // Server sends a response with headers exceeding client's limit
        let large_value = "x".repeat(2048);
        let response = http::Response::builder()
            .status(200)
            .header("x-large", large_value.as_str())
            .body(())
            .unwrap();
        send_resp.send_response(response, true).await.unwrap();

        // Client should reject the oversized headers
        let resp = resp_fut.await_response().await;
        assert!(
            resp.is_err(),
            "client should reject response with oversized headers"
        );
    });
    timeout
        .await
        .expect("security_asymmetric_max_header_list_size timed out");
}
