//! Connection establishment and request/response tests.
//!
//! Tests basic HTTP/2 client-server roundtrips, handshake variants,
//! request/response body handling, HTTP methods, status codes, and headers.

mod common;

use compio_h2::{ClientBuilder, ConnSettings, PingPong, ServerBuilder};
use compio_net::{TcpListener, TcpStream};

/// Test a basic HTTP/2 client-server roundtrip using the raw `handshake()` API.
///
/// Intentionally does NOT use `common::setup()` — this is a smoke test for the
/// low-level `compio_h2::client::handshake` / `compio_h2::server::handshake`
/// functions that the Builder API wraps.
#[compio_macros::test]
async fn h2_roundtrip() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    // Spawn server
    compio_runtime::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut conn = compio_h2::server::handshake(stream).await.unwrap();

        while let Some(result) = conn.accept().await {
            let (request, mut send_response) = result.unwrap();

            let (parts, mut recv_stream) = request.into_parts();
            assert_eq!(parts.method, http::Method::POST);
            assert_eq!(parts.uri.path(), "/test");

            // Read request body
            let mut body = Vec::new();
            while let Some(chunk) = recv_stream.data().await {
                body.extend_from_slice(&chunk.unwrap());
            }
            assert_eq!(body, b"hello server");

            // Send response
            let response = http::Response::builder().status(200).body(()).unwrap();
            let mut send_stream = send_response
                .send_response(response, false)
                .await
                .unwrap()
                .unwrap();

            send_stream
                .send_data(b"hello client".to_vec(), true)
                .await
                .unwrap();
        }
    })
    .detach();

    // Client
    let stream = TcpStream::connect(addr).await.unwrap();
    let (mut send_request, connection) = compio_h2::client::handshake(stream).await.unwrap();

    compio_runtime::spawn(async move {
        if let Err(e) = connection.run().await {
            eprintln!("client connection error: {}", e);
        }
    })
    .detach();

    let request = http::Request::builder()
        .method(http::Method::POST)
        .uri("http://localhost/test")
        .body(())
        .unwrap();

    let (response_future, send_stream) = send_request.send_request(request, false).await.unwrap();

    // Send request body
    let mut send_stream = send_stream.unwrap();
    send_stream
        .send_data(b"hello server".to_vec(), true)
        .await
        .unwrap();

    // Await response
    let response = response_future.await_response().await.unwrap();
    assert_eq!(response.status(), 200);

    let (_, mut recv_stream) = response.into_parts();
    let mut body = Vec::new();
    while let Some(chunk) = recv_stream.data().await {
        body.extend_from_slice(&chunk.unwrap());
    }
    assert_eq!(body, b"hello client");
}

/// Test sending request with END_STREAM in headers (no body) using the raw
/// `handshake()` API.
///
/// Same rationale as `h2_roundtrip` — exercises the low-level handshake path.
#[compio_macros::test]
async fn h2_no_body() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    // Spawn server
    compio_runtime::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut conn = compio_h2::server::handshake(stream).await.unwrap();

        if let Some(result) = conn.accept().await {
            let (request, mut send_response) = result.unwrap();
            let (parts, _recv_stream) = request.into_parts();
            assert_eq!(parts.method, http::Method::GET);

            let response = http::Response::builder().status(200).body(()).unwrap();
            // Send response with no body (end_of_stream = true)
            send_response.send_response(response, true).await.unwrap();
        }
    })
    .detach();

    let stream = TcpStream::connect(addr).await.unwrap();
    let (mut send_request, connection) = compio_h2::client::handshake(stream).await.unwrap();

    compio_runtime::spawn(async move {
        let _ = connection.run().await;
    })
    .detach();

    let request = http::Request::builder()
        .method(http::Method::GET)
        .uri("http://localhost/")
        .body(())
        .unwrap();

    let (response_future, _) = send_request.send_request(request, true).await.unwrap();
    let response = response_future.await_response().await.unwrap();
    assert_eq!(response.status(), 200);
}

#[compio_macros::test]
async fn handshake_default_settings() {
    let (mut client, mut server) = common::setup().await;

    // Simple GET to confirm the connection works
    let (resp_fut, _) = client
        .send_request(common::get_request("/"), true)
        .await
        .unwrap();

    let (req, mut send_resp) = server.accept().await.unwrap().unwrap();
    assert_eq!(req.into_parts().0.method, http::Method::GET);
    send_resp
        .send_response(common::ok_response(), true)
        .await
        .unwrap();

    let resp = resp_fut.await_response().await.unwrap();
    assert_eq!(resp.status(), 200);
}

#[compio_macros::test]
async fn handshake_with_builder() {
    let cb = ClientBuilder::new()
        .initial_window_size(1 << 20)
        .max_frame_size(32768);
    let sb = ServerBuilder::new()
        .initial_window_size(1 << 20)
        .max_frame_size(32768);

    let (mut client, mut server) = common::setup_with_builders(cb, sb).await;

    let (resp_fut, _) = client
        .send_request(common::get_request("/settings"), true)
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
async fn handshake_with_settings_api() {
    // Use low-level handshake_with_settings directly
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let (server_tx, server_rx) = flume::bounded(1);
    compio_runtime::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let settings = ConnSettings::new();
        let ping_pong = PingPong::disabled();
        let conn = compio_h2::server::handshake_with_settings(
            stream,
            settings,
            ping_pong,
            None,
            Default::default(),
        )
        .await
        .unwrap();
        let _ = server_tx.send_async(conn).await;
    })
    .detach();

    let stream = TcpStream::connect(addr).await.unwrap();
    let settings = ConnSettings::new();
    let ping_pong = PingPong::disabled();
    let (mut client, connection) = compio_h2::client::handshake_with_settings(
        stream,
        settings,
        ping_pong,
        None,
        Default::default(),
    )
    .await
    .unwrap();
    compio_runtime::spawn(async move {
        let _ = connection.run().await;
    })
    .detach();

    let mut server = server_rx.recv_async().await.unwrap();

    let (resp_fut, _) = client
        .send_request(common::get_request("/low-level"), true)
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
async fn get_no_body() {
    let (mut client, mut server) = common::setup().await;

    let (resp_fut, send_stream) = client
        .send_request(common::get_request("/no-body"), true)
        .await
        .unwrap();
    assert!(send_stream.is_none()); // end_of_stream=true → no SendStream

    let (req, mut send_resp) = server.accept().await.unwrap().unwrap();
    let (parts, mut recv) = req.into_parts();
    assert_eq!(parts.method, http::Method::GET);
    // No body expected
    assert!(recv.data().await.is_none());

    send_resp
        .send_response(common::ok_response(), true)
        .await
        .unwrap();

    let resp = resp_fut.await_response().await.unwrap();
    assert_eq!(resp.status(), 200);
}

#[compio_macros::test]
async fn post_with_body() {
    let (mut client, mut server) = common::setup().await;

    let (resp_fut, send_stream) = client
        .send_request(common::post_request("/echo"), false)
        .await
        .unwrap();
    let mut send_stream = send_stream.unwrap();
    send_stream
        .send_data(b"request body".to_vec(), true)
        .await
        .unwrap();

    let (req, mut send_resp) = server.accept().await.unwrap().unwrap();
    let (parts, mut recv) = req.into_parts();
    assert_eq!(parts.method, http::Method::POST);
    let body = common::recv_all(&mut recv).await;
    assert_eq!(body, b"request body");

    let mut resp_stream = send_resp
        .send_response(common::ok_response(), false)
        .await
        .unwrap()
        .unwrap();
    resp_stream
        .send_data(b"response body".to_vec(), true)
        .await
        .unwrap();

    let resp = resp_fut.await_response().await.unwrap();
    assert_eq!(resp.status(), 200);
    let (_, mut recv) = resp.into_parts();
    let body = common::recv_all(&mut recv).await;
    assert_eq!(body, b"response body");
}

#[compio_macros::test]
async fn post_empty_body() {
    let (mut client, mut server) = common::setup().await;

    let (resp_fut, send_stream) = client
        .send_request(common::post_request("/empty"), false)
        .await
        .unwrap();
    let mut send_stream = send_stream.unwrap();
    send_stream.send_data(vec![], true).await.unwrap();

    let (req, mut send_resp) = server.accept().await.unwrap().unwrap();
    let (_parts, mut recv) = req.into_parts();
    let body = common::recv_all(&mut recv).await;
    assert!(body.is_empty());

    send_resp
        .send_response(common::ok_response(), true)
        .await
        .unwrap();

    let resp = resp_fut.await_response().await.unwrap();
    assert_eq!(resp.status(), 200);
}

#[compio_macros::test]
async fn various_http_methods() {
    let (mut client, mut server) = common::setup().await;

    for method in &[http::Method::PUT, http::Method::DELETE, http::Method::PATCH] {
        let req = http::Request::builder()
            .method(method.clone())
            .uri("http://localhost/resource")
            .body(())
            .unwrap();

        let (resp_fut, _) = client.send_request(req, true).await.unwrap();

        let (incoming, mut send_resp) = server.accept().await.unwrap().unwrap();
        assert_eq!(incoming.into_parts().0.method, *method);

        send_resp
            .send_response(common::ok_response(), true)
            .await
            .unwrap();

        let resp = resp_fut.await_response().await.unwrap();
        assert_eq!(resp.status(), 200);
    }
}

#[compio_macros::test]
async fn status_codes() {
    let (mut client, mut server) = common::setup().await;

    for code in &[201u16, 204, 400, 404, 500] {
        let (resp_fut, _) = client
            .send_request(common::get_request("/status"), true)
            .await
            .unwrap();

        let (_req, mut send_resp) = server.accept().await.unwrap().unwrap();
        let response = http::Response::builder().status(*code).body(()).unwrap();
        send_resp.send_response(response, true).await.unwrap();

        let resp = resp_fut.await_response().await.unwrap();
        assert_eq!(resp.status(), *code);
    }
}

#[compio_macros::test]
async fn custom_headers_roundtrip() {
    let (mut client, mut server) = common::setup().await;

    let req = http::Request::builder()
        .method(http::Method::GET)
        .uri("http://localhost/headers")
        .header("x-custom-one", "value-one")
        .header("x-custom-two", "value-two")
        .body(())
        .unwrap();

    let (resp_fut, _) = client.send_request(req, true).await.unwrap();

    let (incoming, mut send_resp) = server.accept().await.unwrap().unwrap();
    let (parts, _) = incoming.into_parts();
    assert_eq!(
        parts.headers.get("x-custom-one").unwrap().to_str().unwrap(),
        "value-one"
    );
    assert_eq!(
        parts.headers.get("x-custom-two").unwrap().to_str().unwrap(),
        "value-two"
    );

    let response = http::Response::builder()
        .status(200)
        .header("x-resp-header", "resp-value")
        .body(())
        .unwrap();
    send_resp.send_response(response, true).await.unwrap();

    let resp = resp_fut.await_response().await.unwrap();
    assert_eq!(
        resp.headers()
            .get("x-resp-header")
            .unwrap()
            .to_str()
            .unwrap(),
        "resp-value"
    );
}

#[compio_macros::test]
async fn large_header_value() {
    let (mut client, mut server) = common::setup().await;

    let large_value = "x".repeat(4096);
    let req = http::Request::builder()
        .method(http::Method::GET)
        .uri("http://localhost/large-header")
        .header("x-large", large_value.as_str())
        .body(())
        .unwrap();

    let (resp_fut, _) = client.send_request(req, true).await.unwrap();

    let (incoming, mut send_resp) = server.accept().await.unwrap().unwrap();
    let (parts, _) = incoming.into_parts();
    assert_eq!(
        parts.headers.get("x-large").unwrap().to_str().unwrap(),
        large_value
    );

    send_resp
        .send_response(common::ok_response(), true)
        .await
        .unwrap();

    let resp = resp_fut.await_response().await.unwrap();
    assert_eq!(resp.status(), 200);
}
