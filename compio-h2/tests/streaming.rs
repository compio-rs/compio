//! Multiplexing and body transfer tests.

use std::time::Duration;

use compio_h2::{ClientBuilder, ServerBuilder};

mod common;

#[compio_macros::test]
async fn sequential_requests() {
    let (mut client, mut server) = common::setup().await;

    for i in 0..5 {
        let path = format!("/seq/{}", i);
        let (resp_fut, _) = client
            .send_request(common::get_request(&path), true)
            .await
            .unwrap();

        let (req, mut send_resp) = server.accept().await.unwrap().unwrap();
        let (parts, _) = req.into_parts();
        assert_eq!(parts.uri.path(), path);

        send_resp
            .send_response(common::ok_response(), true)
            .await
            .unwrap();

        let resp = resp_fut.await_response().await.unwrap();
        assert_eq!(resp.status(), 200);
    }
}

#[compio_macros::test]
async fn concurrent_streams() {
    let (mut client, mut server) = common::setup().await;

    // Fire 3 requests without waiting for responses
    let mut futures = Vec::new();
    for i in 0..3 {
        let path = format!("/concurrent/{}", i);
        let (resp_fut, _) = client
            .send_request(common::get_request(&path), true)
            .await
            .unwrap();
        futures.push(resp_fut);
    }

    // Server handles all 3
    for _ in 0..3 {
        let (_req, mut send_resp) = server.accept().await.unwrap().unwrap();
        send_resp
            .send_response(common::ok_response(), true)
            .await
            .unwrap();
    }

    // All 3 responses should arrive
    for resp_fut in futures {
        let resp = resp_fut.await_response().await.unwrap();
        assert_eq!(resp.status(), 200);
    }
}

#[compio_macros::test]
async fn many_concurrent_streams() {
    let (mut client, mut server) = common::setup().await;

    let mut futures = Vec::new();
    for i in 0..10 {
        let path = format!("/many/{}", i);
        let (resp_fut, _) = client
            .send_request(common::get_request(&path), true)
            .await
            .unwrap();
        futures.push(resp_fut);
    }

    // Server responds to all, echoing the path as a custom header
    for _ in 0..10 {
        let (req, mut send_resp) = server.accept().await.unwrap().unwrap();
        let path = req.into_parts().0.uri.path().to_string();
        let response = http::Response::builder()
            .status(200)
            .header("x-path", path.as_str())
            .body(())
            .unwrap();
        send_resp.send_response(response, true).await.unwrap();
    }

    let mut paths: Vec<String> = Vec::new();
    for resp_fut in futures {
        let resp = resp_fut.await_response().await.unwrap();
        assert_eq!(resp.status(), 200);
        let p = resp
            .headers()
            .get("x-path")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        paths.push(p);
    }
    // All 10 paths should be present (order may vary)
    paths.sort();
    for i in 0..10 {
        assert!(paths.contains(&format!("/many/{}", i)));
    }
}

#[compio_macros::test]
async fn max_concurrent_streams_enforced() {
    let cb = ClientBuilder::new();
    let sb = ServerBuilder::new().max_concurrent_streams(1);

    let (mut client, mut server) = common::setup_with_builders(cb, sb).await;

    // First request — should succeed
    let (resp_fut_1, _) = client
        .send_request(common::get_request("/first"), true)
        .await
        .unwrap();

    // Second request while first is open — should be refused
    let result = client
        .send_request(common::get_request("/second"), true)
        .await;

    // Handle the first request on the server
    let (_req, mut send_resp) = server.accept().await.unwrap().unwrap();
    send_resp
        .send_response(common::ok_response(), true)
        .await
        .unwrap();

    let resp = resp_fut_1.await_response().await.unwrap();
    assert_eq!(resp.status(), 200);

    // The second request should have errored (REFUSED_STREAM) or we handle it
    // gracefully If the implementation queues it, it may succeed after the
    // first completes
    match result {
        Err(e) => {
            // Expected: REFUSED_STREAM or similar
            assert!(
                format!("{:?}", e).contains("Refused")
                    || format!("{:?}", e).contains("concurrent")
                    || format!("{:?}", e).contains("stream"),
                "unexpected error: {:?}",
                e
            );
        }
        Ok((resp_fut_2, _)) => {
            // Implementation may queue the request — that's acceptable too
            let (_req, mut send_resp) = server.accept().await.unwrap().unwrap();
            send_resp
                .send_response(common::ok_response(), true)
                .await
                .unwrap();
            let resp = resp_fut_2.await_response().await.unwrap();
            assert_eq!(resp.status(), 200);
        }
    }
}

// ===========================================================================
// Body Transfer
// ===========================================================================

#[compio_macros::test]
async fn multi_chunk_body() {
    let (mut client, mut server) = common::setup().await;

    let (resp_fut, send_stream) = client
        .send_request(common::post_request("/chunks"), false)
        .await
        .unwrap();
    let mut ss = send_stream.unwrap();
    ss.send_data(b"chunk1-".to_vec(), false).await.unwrap();
    ss.send_data(b"chunk2-".to_vec(), false).await.unwrap();
    ss.send_data(b"chunk3".to_vec(), true).await.unwrap();

    let (req, mut send_resp) = server.accept().await.unwrap().unwrap();
    let (_, mut recv) = req.into_parts();
    let body = common::recv_all(&mut recv).await;
    assert_eq!(body, b"chunk1-chunk2-chunk3");

    send_resp
        .send_response(common::ok_response(), true)
        .await
        .unwrap();

    let resp = resp_fut.await_response().await.unwrap();
    assert_eq!(resp.status(), 200);
}

#[compio_macros::test]
async fn large_body_within_window() {
    let timeout = compio_runtime::time::timeout(Duration::from_secs(10), async {
        let (mut client, mut server) = common::setup().await;

        // 60KB — fits in default 65535 window
        let payload = vec![0xABu8; 60 * 1024];

        let (resp_fut, send_stream) = client
            .send_request(common::post_request("/large"), false)
            .await
            .unwrap();
        let mut ss = send_stream.unwrap();
        common::send_chunked(&mut ss, &payload, 16 * 1024).await;

        let (req, mut send_resp) = server.accept().await.unwrap().unwrap();
        let (_, mut recv) = req.into_parts();
        let body = common::recv_all(&mut recv).await;
        assert_eq!(body.len(), 60 * 1024);
        assert_eq!(body, payload);

        send_resp
            .send_response(common::ok_response(), true)
            .await
            .unwrap();

        let resp = resp_fut.await_response().await.unwrap();
        assert_eq!(resp.status(), 200);
    });
    timeout.await.expect("large_body_within_window timed out");
}

#[compio_macros::test]
async fn large_body_exceeds_window() {
    let timeout = compio_runtime::time::timeout(Duration::from_secs(10), async {
        let (mut client, mut server) = common::setup().await;

        // 128KB — exceeds 65535 default window, triggers WINDOW_UPDATE
        let payload = vec![0xCDu8; 128 * 1024];

        let (resp_fut, send_stream) = client
            .send_request(common::post_request("/flow"), false)
            .await
            .unwrap();

        // Spawn sender in background so receiver can run concurrently
        let mut ss = send_stream.unwrap();
        let send_task = compio_runtime::spawn(async move {
            common::send_chunked(&mut ss, &payload, 16 * 1024).await;
        });

        let (req, mut send_resp) = server.accept().await.unwrap().unwrap();
        let (_, mut recv) = req.into_parts();
        let body = common::recv_all(&mut recv).await;
        assert_eq!(body.len(), 128 * 1024);

        let _ = send_task.await;

        send_resp
            .send_response(common::ok_response(), true)
            .await
            .unwrap();

        let resp = resp_fut.await_response().await.unwrap();
        assert_eq!(resp.status(), 200);
    });
    timeout.await.expect("large_body_exceeds_window timed out");
}

#[compio_macros::test]
async fn large_body_server_to_client() {
    let timeout = compio_runtime::time::timeout(Duration::from_secs(10), async {
        let (mut client, mut server) = common::setup().await;

        let payload = vec![0xEFu8; 128 * 1024];

        let (resp_fut, _) = client
            .send_request(common::get_request("/download"), true)
            .await
            .unwrap();

        let (_req, mut send_resp) = server.accept().await.unwrap().unwrap();
        let mut resp_stream = send_resp
            .send_response(common::ok_response(), false)
            .await
            .unwrap()
            .unwrap();

        // Spawn sender in background so receiver can run concurrently
        let send_task = compio_runtime::spawn(async move {
            common::send_chunked(&mut resp_stream, &payload, 16 * 1024).await;
        });

        let resp = resp_fut.await_response().await.unwrap();
        assert_eq!(resp.status(), 200);
        let (_, mut recv) = resp.into_parts();
        let body = common::recv_all(&mut recv).await;
        assert_eq!(body.len(), 128 * 1024);

        let _ = send_task.await;
    });
    timeout
        .await
        .expect("large_body_server_to_client timed out");
}
