//! Flow control tests — window sizes, capacity, and partial DATA splitting.

use std::time::Duration;

use bytes::Bytes;
use compio_h2::{ClientBuilder, ServerBuilder};

mod common;

#[compio_macros::test]
async fn custom_initial_window_size() {
    let timeout = compio_runtime::time::timeout(Duration::from_secs(10), async {
        let cb = ClientBuilder::new().initial_window_size(16384);
        let sb = ServerBuilder::new().initial_window_size(16384);
        let (mut client, mut server) = common::setup_with_builders(cb, sb).await;

        // 32KB — 2x the window, so WINDOW_UPDATE must fire
        let payload = vec![0x42u8; 32 * 1024];

        let (resp_fut, send_stream) = client
            .send_request(common::post_request("/flow-custom"), false)
            .await
            .unwrap();

        // Spawn sender in background so receiver can run concurrently
        let mut ss = send_stream.unwrap();
        let send_task = compio_runtime::spawn(async move {
            common::send_chunked(&mut ss, &payload, 4096).await;
        });

        let (req, mut send_resp) = server.accept().await.unwrap().unwrap();
        let (_, mut recv) = req.into_parts();
        let body = common::recv_all(&mut recv).await;
        assert_eq!(body.len(), 32 * 1024);

        let _ = send_task.await;

        send_resp
            .send_response(common::ok_response(), true)
            .await
            .unwrap();

        let resp = resp_fut.await_response().await.unwrap();
        assert_eq!(resp.status(), 200);
    });
    timeout.await.expect("custom_initial_window_size timed out");
}

#[compio_macros::test]
async fn small_window_large_transfer() {
    let timeout = compio_runtime::time::timeout(Duration::from_secs(15), async {
        let cb = ClientBuilder::new().initial_window_size(1024);
        let sb = ServerBuilder::new().initial_window_size(1024);
        let (mut client, mut server) = common::setup_with_builders(cb, sb).await;

        // 8KB with 1024 window — many WINDOW_UPDATE rounds
        let payload = vec![0x77u8; 8 * 1024];

        let (resp_fut, send_stream) = client
            .send_request(common::post_request("/small-window"), false)
            .await
            .unwrap();

        // Spawn sender in background so receiver can run concurrently
        let mut ss = send_stream.unwrap();
        let send_task = compio_runtime::spawn(async move {
            common::send_chunked(&mut ss, &payload, 512).await;
        });

        let (req, mut send_resp) = server.accept().await.unwrap().unwrap();
        let (_, mut recv) = req.into_parts();
        let body = common::recv_all(&mut recv).await;
        assert_eq!(body.len(), 8 * 1024);

        let _ = send_task.await;

        send_resp
            .send_response(common::ok_response(), true)
            .await
            .unwrap();

        let resp = resp_fut.await_response().await.unwrap();
        assert_eq!(resp.status(), 200);
    });
    timeout
        .await
        .expect("small_window_large_transfer timed out");
}

#[compio_macros::test]
async fn connection_level_flow_control() {
    let timeout = compio_runtime::time::timeout(Duration::from_secs(15), async {
        let (mut client, mut server) = common::setup().await;

        // 3 streams x 30KB = 90KB > 65535 connection window
        let payload = vec![0x99u8; 30 * 1024];

        let mut resp_futures = Vec::new();
        let mut send_streams = Vec::new();

        for i in 0..3 {
            let path = format!("/flow-conn/{}", i);
            let (resp_fut, ss) = client
                .send_request(common::post_request(&path), false)
                .await
                .unwrap();
            resp_futures.push(resp_fut);
            send_streams.push(ss.unwrap());
        }

        // Send data on all 3 streams in chunks
        for ss in send_streams.iter_mut() {
            common::send_chunked(ss, &payload, 8192).await;
        }

        // Server receives all 3
        for _ in 0..3 {
            let (req, mut send_resp) = server.accept().await.unwrap().unwrap();
            let (_, mut recv) = req.into_parts();
            let body = common::recv_all(&mut recv).await;
            assert_eq!(body.len(), 30 * 1024);

            send_resp
                .send_response(common::ok_response(), true)
                .await
                .unwrap();
        }

        for resp_fut in resp_futures {
            let resp = resp_fut.await_response().await.unwrap();
            assert_eq!(resp.status(), 200);
        }
    });
    timeout
        .await
        .expect("connection_level_flow_control timed out");
}

#[compio_macros::test]
async fn bidirectional_data_flow() {
    let timeout = compio_runtime::time::timeout(Duration::from_secs(10), async {
        let (mut client, mut server) = common::setup().await;

        let client_payload = vec![0xAAu8; 16 * 1024];
        let server_payload = vec![0xBBu8; 16 * 1024];

        let (resp_fut, send_stream) = client
            .send_request(common::post_request("/bidi"), false)
            .await
            .unwrap();
        let mut client_ss = send_stream.unwrap();
        common::send_chunked(&mut client_ss, &client_payload, 4096).await;

        let (req, mut send_resp) = server.accept().await.unwrap().unwrap();
        let (_, mut recv) = req.into_parts();
        let body = common::recv_all(&mut recv).await;
        assert_eq!(body.len(), 16 * 1024);

        // Server sends response body
        let mut resp_ss = send_resp
            .send_response(common::ok_response(), false)
            .await
            .unwrap()
            .unwrap();

        common::send_chunked(&mut resp_ss, &server_payload, 4096).await;

        let resp = resp_fut.await_response().await.unwrap();
        assert_eq!(resp.status(), 200);
        let (_, mut recv) = resp.into_parts();
        let body = common::recv_all(&mut recv).await;
        assert_eq!(body.len(), 16 * 1024);
        assert_eq!(body, server_payload);
    });
    timeout.await.expect("bidirectional_data_flow timed out");
}

// ---------------------------------------------------------------------------
// Runtime window adjustment
// ---------------------------------------------------------------------------

#[compio_macros::test]
async fn set_target_window_size_server() {
    let (mut client, mut server) = common::setup().await;

    // Increase the server's connection receive window at runtime
    server.set_target_window_size(1 << 20).await.unwrap();

    // Verify requests still work after adjusting the window
    let (resp_fut, _) = client
        .send_request(common::get_request("/window-test"), true)
        .await
        .unwrap();

    let (_req, mut send_resp) = server.accept().await.unwrap().unwrap();
    send_resp
        .send_response(common::ok_response(), true)
        .await
        .unwrap();

    let resp = resp_fut.await_response().await.unwrap();
    assert_eq!(resp.status(), 200);

    // Now test with body: server sends response data (server->client direction
    // is unaffected by server's recv window, but verifies no corruption)
    let (resp_fut2, _) = client
        .send_request(common::get_request("/window-test-2"), true)
        .await
        .unwrap();

    let (_req2, mut send_resp2) = server.accept().await.unwrap().unwrap();
    let mut ss = send_resp2
        .send_response(common::ok_response(), false)
        .await
        .unwrap()
        .unwrap();
    let payload = vec![0xAA; 16_000];
    ss.send_data(payload.clone(), true).await.unwrap();

    let resp2 = resp_fut2.await_response().await.unwrap();
    assert_eq!(resp2.status(), 200);
    let body = common::recv_all(&mut resp2.into_body()).await;
    assert_eq!(body, payload);
}

#[compio_macros::test]
async fn set_target_window_size_client() {
    let (mut client, mut server) = common::setup().await;

    // Increase the client's connection receive window at runtime
    client.set_target_window_size(1 << 20).await.unwrap();

    // Verify requests still work — server sends response body to test
    // that the client's increased window doesn't break anything
    let (resp_fut, _) = client
        .send_request(common::get_request("/window-client"), true)
        .await
        .unwrap();

    let (_req, mut send_resp) = server.accept().await.unwrap().unwrap();
    let mut ss = send_resp
        .send_response(common::ok_response(), false)
        .await
        .unwrap()
        .unwrap();
    let payload = vec![0xBB; 16_000];
    ss.send_data(payload.clone(), true).await.unwrap();

    let resp = resp_fut.await_response().await.unwrap();
    assert_eq!(resp.status(), 200);
    let body = common::recv_all(&mut resp.into_body()).await;
    assert_eq!(body, payload);
}

#[compio_macros::test]
async fn set_initial_window_size_server() {
    let (mut client, mut server) = common::setup().await;

    // Change the server's stream initial window size via SETTINGS
    server.set_initial_window_size(1 << 17).await.unwrap();

    // Open a new stream — it should use the updated initial window size
    let (resp_fut, send_stream) = client
        .send_request(common::post_request("/init-window"), false)
        .await
        .unwrap();

    let payload = vec![0xCC; 16_000];
    let payload2 = payload.clone();
    let send_task = compio_runtime::spawn(async move {
        let mut ss = send_stream.unwrap();
        ss.send_data(payload2, true).await.unwrap();
    });

    let (req, mut send_resp) = server.accept().await.unwrap().unwrap();
    assert_eq!(req.method(), http::Method::POST);
    let body = common::recv_all(&mut req.into_body()).await;
    assert_eq!(body, payload);

    let _ = send_task.await;

    send_resp
        .send_response(common::ok_response(), true)
        .await
        .unwrap();
    let resp = resp_fut.await_response().await.unwrap();
    assert_eq!(resp.status(), 200);
}

#[compio_macros::test]
async fn set_initial_window_size_client() {
    let (mut client, mut server) = common::setup().await;

    // Change the client's stream initial window size via SETTINGS
    client.set_initial_window_size(1 << 17).await.unwrap();

    // Open a stream and exchange data
    let (resp_fut, _) = client
        .send_request(common::get_request("/init-window-client"), true)
        .await
        .unwrap();

    let (_req, mut send_resp) = server.accept().await.unwrap().unwrap();
    let mut ss = send_resp
        .send_response(common::ok_response(), false)
        .await
        .unwrap()
        .unwrap();
    let payload = vec![0xDD; 16_000];
    ss.send_data(payload.clone(), true).await.unwrap();

    let resp = resp_fut.await_response().await.unwrap();
    assert_eq!(resp.status(), 200);
    let body = common::recv_all(&mut resp.into_body()).await;
    assert_eq!(body, payload);
}

#[compio_macros::test]
async fn runtime_window_adjustment_with_existing_streams() {
    let (mut client, mut server) = common::setup().await;

    // Open a stream first
    let (resp_fut, _) = client
        .send_request(common::get_request("/existing"), true)
        .await
        .unwrap();

    let (_req, mut send_resp) = server.accept().await.unwrap().unwrap();

    // Now adjust the server's initial window size while the stream is open
    // This should apply the delta to the existing stream's recv window
    server.set_initial_window_size(1 << 17).await.unwrap();

    // Send response body — should still work
    let mut ss = send_resp
        .send_response(common::ok_response(), false)
        .await
        .unwrap()
        .unwrap();
    let payload = vec![0xEE; 8_000];
    ss.send_data(payload.clone(), true).await.unwrap();

    let resp = resp_fut.await_response().await.unwrap();
    assert_eq!(resp.status(), 200);
    let body = common::recv_all(&mut resp.into_body()).await;
    assert_eq!(body, payload);
}

// ---------------------------------------------------------------------------
// reserve_capacity / poll_capacity
// ---------------------------------------------------------------------------

#[compio_macros::test]
async fn reserve_capacity_grants_capacity() {
    let (mut client, mut server) = common::setup().await;

    let (resp_fut, send_stream) = client
        .send_request(common::post_request("/reserve"), false)
        .await
        .unwrap();
    let mut ss = send_stream.unwrap();

    // Reserve 1024 bytes of send capacity
    ss.reserve_capacity(1024).await.unwrap();
    assert!(ss.capacity() >= 1024);

    // Send data within the reservation
    ss.send_data(vec![0xAA; 512], false).await.unwrap();
    ss.send_data(vec![0xBB; 512], true).await.unwrap();

    // Server side: accept and drain
    let (req, mut send_resp) = server.accept().await.unwrap().unwrap();
    let body = common::recv_all(&mut req.into_body()).await;
    assert_eq!(body.len(), 1024);

    let mut resp_ss = send_resp
        .send_response(common::ok_response(), false)
        .await
        .unwrap()
        .unwrap();
    resp_ss.send_data(b"ok".to_vec(), true).await.unwrap();

    let resp = resp_fut.await_response().await.unwrap();
    assert_eq!(resp.status(), 200);
}

#[compio_macros::test]
async fn poll_capacity_returns_available() {
    let (mut client, mut server) = common::setup().await;

    let (resp_fut, send_stream) = client
        .send_request(common::post_request("/poll-cap"), false)
        .await
        .unwrap();
    let mut ss = send_stream.unwrap();

    // poll_capacity should return some available capacity (default window = 65535)
    let cap = ss.poll_capacity().await;
    assert!(cap.is_some());
    let cap = cap.unwrap().unwrap();
    assert!(cap > 0);

    ss.send_data(vec![0xCC; 100], true).await.unwrap();

    // Server side
    let (req, mut send_resp) = server.accept().await.unwrap().unwrap();
    let body = common::recv_all(&mut req.into_body()).await;
    assert_eq!(body.len(), 100);

    send_resp
        .send_response(common::ok_response(), true)
        .await
        .unwrap();
    let resp = resp_fut.await_response().await.unwrap();
    assert_eq!(resp.status(), 200);
}

// ---------------------------------------------------------------------------
// Send-side flow control — partial DATA splitting
// ---------------------------------------------------------------------------

/// Validates the deadlock fix: a single send_data() call with payload > initial
/// window (128KB > 65535) must complete without deadlock thanks to partial
/// flow control sends.
#[compio_macros::test]
async fn large_single_send_without_chunking() {
    let timeout = compio_runtime::time::timeout(Duration::from_secs(10), async {
        let (mut client, mut server) = common::setup().await;

        // 128KB — exceeds default 65535 window in a single send_data call
        let payload = vec![0xDDu8; 128 * 1024];

        let (resp_fut, send_stream) = client
            .send_request(common::post_request("/single-large"), false)
            .await
            .unwrap();

        // Single send_data call (no chunking!) — tests partial flow control queuing
        let mut ss = send_stream.unwrap();
        let send_payload = payload.clone();
        let send_task = compio_runtime::spawn(async move {
            ss.send_data(send_payload, true).await.unwrap();
        });

        let (req, mut send_resp) = server.accept().await.unwrap().unwrap();
        let (_, mut recv) = req.into_parts();
        let body = common::recv_all(&mut recv).await;
        assert_eq!(body.len(), 128 * 1024);
        assert_eq!(body, payload);

        let _ = send_task.await;

        send_resp
            .send_response(common::ok_response(), true)
            .await
            .unwrap();

        let resp = resp_fut.await_response().await.unwrap();
        assert_eq!(resp.status(), 200);
    });
    timeout
        .await
        .expect("large_single_send_without_chunking timed out (possible deadlock)");
}

/// Validates frame splitting with a custom max_frame_size: configure 8192,
/// send 32KB payload, verify all data arrives intact.
#[compio_macros::test]
async fn custom_max_frame_size_large_payload() {
    let timeout = compio_runtime::time::timeout(Duration::from_secs(10), async {
        // Both sides advertise max_frame_size=16384 (the RFC minimum /
        // default). The 48KB payload must be split into 3 DATA frames.
        let cb = ClientBuilder::new().max_frame_size(16384);
        let sb = ServerBuilder::new().max_frame_size(16384);

        let (mut client, mut server) = common::setup_with_builders(cb, sb).await;

        // 48KB payload — should be split into 3 × 16384-byte DATA frames
        let payload = vec![0xEEu8; 48 * 1024];

        let (resp_fut, send_stream) = client
            .send_request(common::post_request("/frame-split"), false)
            .await
            .unwrap();

        // Single send_data call — exercises frame splitting in cmd_send_data
        let mut ss = send_stream.unwrap();
        let send_payload = payload.clone();
        let send_task = compio_runtime::spawn(async move {
            ss.send_data(send_payload, true).await.unwrap();
        });

        let (req, mut send_resp) = server.accept().await.unwrap().unwrap();
        let (_, mut recv) = req.into_parts();
        let body = common::recv_all(&mut recv).await;
        assert_eq!(body.len(), 48 * 1024);
        assert_eq!(body, payload);

        let _ = send_task.await;

        send_resp
            .send_response(common::ok_response(), true)
            .await
            .unwrap();

        let resp = resp_fut.await_response().await.unwrap();
        assert_eq!(resp.status(), 200);
    });
    timeout
        .await
        .expect("custom_max_frame_size_large_payload timed out");
}

#[compio_macros::test]
async fn tiny_window_partial_data() {
    // INITIAL_WINDOW_SIZE=16 forces the partial-send path in try_send_or_queue:
    // each partial send carries at most 16 bytes, exercising the remainder /
    // re-queue loop in flush_pending_sends for a multi-byte payload.
    let timeout = compio_runtime::time::timeout(Duration::from_secs(15), async {
        let cb = ClientBuilder::new().initial_window_size(16);
        let sb = ServerBuilder::new().initial_window_size(16);
        let (mut client, mut server) = common::setup_with_builders(cb, sb).await;

        // 128 bytes with 16-byte window → 8 WINDOW_UPDATE rounds
        let payload = vec![0xABu8; 128];

        let (resp_fut, send_stream) = client
            .send_request(common::post_request("/tiny-window"), false)
            .await
            .unwrap();

        // Spawn sender so receiver can run concurrently (avoids deadlock)
        let mut ss = send_stream.unwrap();
        let payload_clone = payload.clone();
        let send_task = compio_runtime::spawn(async move {
            common::send_chunked(&mut ss, &payload_clone, 16).await;
        });

        let (req, mut send_resp) = server.accept().await.unwrap().unwrap();
        let (_, mut recv) = req.into_parts();
        let body = common::recv_all(&mut recv).await;
        assert_eq!(body, payload, "client->server payload mismatch");

        let _ = send_task.await;

        // Server sends response body (also exercises partial DATA path)
        let response_payload = vec![0xCDu8; 96];
        let mut ss2 = send_resp
            .send_response(common::ok_response(), false)
            .await
            .unwrap()
            .unwrap();
        let rp = response_payload.clone();
        let send_task2 = compio_runtime::spawn(async move {
            common::send_chunked(&mut ss2, &rp, 16).await;
        });

        let resp = resp_fut.await_response().await.unwrap();
        assert_eq!(resp.status(), 200);
        let (_, mut recv2) = resp.into_parts();
        let body2 = common::recv_all(&mut recv2).await;
        assert_eq!(body2, response_payload, "server->client payload mismatch");

        let _ = send_task2.await;
    });
    timeout.await.expect("tiny_window_partial_data timed out");
}

// ---------------------------------------------------------------------------
// Security — flow control window overflow
// ---------------------------------------------------------------------------

/// Flow control window overflow: INITIAL_WINDOW_SIZE set to i32::MAX
/// should work without overflow. Verifies the implementation handles
/// the maximum valid window size correctly.
///
/// Note: Triggering a FLOW_CONTROL_ERROR via the public API is difficult
/// because the implementation correctly clamps WINDOW_UPDATE values.
/// The frame-level checks in FlowControl::credit() and the
/// INITIAL_WINDOW_SIZE delta logic prevent overflow from reaching the
/// wire. This test verifies the maximum valid window size works and
/// that a large payload transfer succeeds under those conditions.
#[compio_macros::test]
async fn security_flow_control_window_overflow() {
    let timeout = compio_runtime::time::timeout(Duration::from_secs(10), async {
        // Use max valid initial window size (2^31 - 1)
        let max_window = i32::MAX as u32;
        let cb = ClientBuilder::new().initial_window_size(max_window);
        let sb = ServerBuilder::new().initial_window_size(max_window);

        let (mut client, mut server) = common::setup_with_builders(cb, sb).await;

        // Send a moderately large payload — with max window, it should
        // flow through without any WINDOW_UPDATE round-trips needed.
        let payload = vec![0xBBu8; 64 * 1024];

        let (resp_fut, send_stream) = client
            .send_request(common::post_request("/flow-overflow"), false)
            .await
            .unwrap();

        let mut ss = send_stream.unwrap();
        let payload_clone = payload.clone();
        let send_task = compio_runtime::spawn(async move {
            ss.send_data(payload_clone, true).await.unwrap();
        });

        let (req, mut send_resp) = server.accept().await.unwrap().unwrap();
        let (_, mut recv) = req.into_parts();
        let body = common::recv_all(&mut recv).await;
        assert_eq!(body.len(), 64 * 1024);
        assert_eq!(body, payload);

        let _ = send_task.await;

        send_resp
            .send_response(common::ok_response(), true)
            .await
            .unwrap();

        let resp = resp_fut.await_response().await.unwrap();
        assert_eq!(resp.status(), 200);
    });
    timeout
        .await
        .expect("security_flow_control_window_overflow timed out");
}

// ---------------------------------------------------------------------------
// max_frame_size propagation
// ---------------------------------------------------------------------------

/// Verify that max_frame_size propagates to FrameReader: a server configured
/// with max_frame_size=32768 accepts a payload that requires >16384-byte DATA
/// frames from the client (which also advertises 32768).
#[compio_macros::test]
async fn max_frame_size_propagated_to_reader() {
    let timeout = compio_runtime::time::timeout(Duration::from_secs(10), async {
        let larger = 32_768u32;
        let cb = ClientBuilder::new().max_frame_size(larger);
        let sb = ServerBuilder::new().max_frame_size(larger);

        let (mut client, mut server) = common::setup_with_builders(cb, sb).await;

        // 24KB payload — fits in one frame at 32768 but would be split at 16384.
        // If the reader still enforced the default 16384 limit this would fail.
        let payload = vec![0xCCu8; 24 * 1024];

        let (resp_fut, send_stream) = client
            .send_request(common::post_request("/large-frame"), false)
            .await
            .unwrap();

        let mut ss = send_stream.unwrap();
        let send_payload = payload.clone();
        let send_task = compio_runtime::spawn(async move {
            ss.send_data(send_payload, true).await.unwrap();
        });

        let (req, mut send_resp) = server.accept().await.unwrap().unwrap();
        let (_, mut recv) = req.into_parts();
        let body = common::recv_all(&mut recv).await;
        assert_eq!(body.len(), 24 * 1024);
        assert_eq!(body, payload);

        let _ = send_task.await;

        send_resp
            .send_response(common::ok_response(), true)
            .await
            .unwrap();

        let resp = resp_fut.await_response().await.unwrap();
        assert_eq!(resp.status(), 200);
    });
    timeout
        .await
        .expect("max_frame_size_propagated_to_reader timed out");
}

#[compio_macros::test]
async fn max_send_buffer_size_enforced() {
    // Verify that send_data rejects when pending bytes exceed max_send_buffer_size.
    // Strategy: use a tiny initial window on the server side so the client's stream
    // send window is small. Then send more data than fits in both the window and
    // the send buffer.
    let timeout = compio_runtime::time::timeout(Duration::from_secs(10), async {
        let cb = ClientBuilder::new().max_send_buffer_size(100);
        let sb = ServerBuilder::new().initial_window_size(1);
        let (mut client, mut server) = common::setup_with_builders(cb, sb).await;

        // Server: accept but never read data — keeps flow control blocked
        compio_runtime::spawn(async move {
            while let Some(result) = server.accept().await {
                let (_req, _send_resp) = result.unwrap();
            }
        })
        .detach();

        // Yield to let the IO driver process the SETTINGS exchange
        compio_runtime::time::sleep(Duration::from_millis(50)).await;

        let req = http::Request::builder()
            .method(http::Method::POST)
            .uri("http://localhost/test")
            .body(())
            .unwrap();
        let (_resp_fut, send_stream) = client.send_request(req, false).await.unwrap();
        let mut ss = send_stream.unwrap();

        // Send 200 bytes. With a 1-byte stream send window, encode_data can only
        // send 1 byte via fast path → returns Ok(false). Then 200 bytes try to
        // queue but 200 > max_send_buffer_size(100) → error.
        let result = ss.send_data(Bytes::from(vec![0u8; 200]), true).await;

        assert!(result.is_err(), "200 bytes should exceed 100-byte send buffer limit");
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("send buffer full"),
            "error should mention send buffer: {}",
            err_msg
        );
    });
    timeout
        .await
        .expect("max_send_buffer_size_enforced timed out");
}
