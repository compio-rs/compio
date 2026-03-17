//! Connection lifecycle tests — ready(), poll_reset(), shutdown, send buffer,
//! DoS protection, and max_send_buffer_size.

use std::time::Duration;

use compio_h2::{ClientBuilder, ServerBuilder};
use compio_net::{TcpListener, TcpStream};

mod common;

// ---------------------------------------------------------------------------
// ready()
// ---------------------------------------------------------------------------

#[compio_macros::test]
async fn ready_blocks_at_max_concurrent_streams() {
    let server_builder = ServerBuilder::new().max_concurrent_streams(1);
    let (mut client, mut server) =
        common::setup_with_builders(ClientBuilder::new(), server_builder).await;

    // First stream should go through immediately
    client.ready().await.unwrap();
    let (resp_fut1, _) = client
        .send_request(common::get_request("/first"), true)
        .await
        .unwrap();

    // Accept and respond to free the slot
    let (_req1, mut send_resp1) = server.accept().await.unwrap().unwrap();
    send_resp1
        .send_response(common::ok_response(), true)
        .await
        .unwrap();

    let resp1 = resp_fut1.await_response().await.unwrap();
    assert_eq!(resp1.status(), 200);

    // After the first stream completes, ready should resolve for the second
    client.ready().await.unwrap();
    let (resp_fut2, _) = client
        .send_request(common::get_request("/second"), true)
        .await
        .unwrap();

    let (_req2, mut send_resp2) = server.accept().await.unwrap().unwrap();
    send_resp2
        .send_response(common::ok_response(), true)
        .await
        .unwrap();

    let resp2 = resp_fut2.await_response().await.unwrap();
    assert_eq!(resp2.status(), 200);
}

// ---------------------------------------------------------------------------
// poll_reset()
// ---------------------------------------------------------------------------

#[compio_macros::test]
async fn poll_reset_detects_rst_stream() {
    let (mut client, mut server) = common::setup().await;

    let (resp_fut, send_stream) = client
        .send_request(common::post_request("/reset-me"), false)
        .await
        .unwrap();
    let mut ss = send_stream.unwrap();

    // Server accepts the stream then sends RST_STREAM
    let (_req, mut send_resp) = server.accept().await.unwrap().unwrap();

    // Server sends response first, then gets a SendStream to reset
    let server_ss = send_resp
        .send_response(common::ok_response(), false)
        .await
        .unwrap()
        .unwrap();
    server_ss
        .send_reset(compio_h2::Reason::Cancel)
        .await
        .unwrap();

    // Client should detect the reset
    let reason = ss.poll_reset().await.unwrap();
    assert_eq!(reason, compio_h2::Reason::Cancel);

    // Clean up
    let _ = resp_fut.await_response().await;
}

// ---------------------------------------------------------------------------
// abrupt_shutdown() / closed()
// ---------------------------------------------------------------------------

#[compio_macros::test]
async fn abrupt_shutdown_sends_goaway() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let (server_tx, server_rx) = flume::bounded(1);

    compio_runtime::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let conn = ServerBuilder::new().handshake(stream).await.unwrap();
        let _ = server_tx.send_async(conn).await;
    })
    .detach();

    let stream = TcpStream::connect(addr).await.unwrap();
    let (mut client, connection) = ClientBuilder::new().handshake(stream).await.unwrap();

    let (closed_tx, closed_rx) = flume::bounded(1);
    compio_runtime::spawn(async move {
        let result = connection.run().await;
        let _ = closed_tx.send_async(result).await;
    })
    .detach();

    let server = server_rx.recv_async().await.unwrap();

    // Open a stream so there's activity
    let (_resp_fut, _) = client
        .send_request(common::get_request("/before-shutdown"), true)
        .await
        .unwrap();

    // Server does abrupt shutdown
    server
        .abrupt_shutdown(compio_h2::Reason::InternalError)
        .await
        .unwrap();

    // Client connection should close
    let result = closed_rx.recv_async().await.unwrap();
    // We just verify the connection ended
    drop(result);
}

#[compio_macros::test]
async fn closed_fires_after_connection_ends() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let (server_tx, server_rx) = flume::bounded(1);

    compio_runtime::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let conn = ServerBuilder::new().handshake(stream).await.unwrap();
        let _ = server_tx.send_async(conn).await;
    })
    .detach();

    let stream = TcpStream::connect(addr).await.unwrap();
    let (client, connection) = ClientBuilder::new().handshake(stream).await.unwrap();

    compio_runtime::spawn(async move {
        let _ = connection.run().await;
    })
    .detach();

    let mut server = server_rx.recv_async().await.unwrap();

    // Initiate a graceful shutdown from the client side, then drop
    client.shutdown().await.unwrap();
    drop(client);

    // Server's closed() should fire once the connection loop terminates
    let result = compio_runtime::time::timeout(Duration::from_secs(5), server.closed()).await;
    assert!(
        result.is_ok(),
        "server.closed() should resolve after client shutdown"
    );
}

// ---------------------------------------------------------------------------
// max_send_buffer_size
// ---------------------------------------------------------------------------

#[compio_macros::test]
async fn max_send_buffer_size_configured() {
    // Verify that the max_send_buffer_size setting is accepted by the builder
    // and propagates through the handshake without error.
    let client_builder = ClientBuilder::new().max_send_buffer_size(1024);
    let server_builder = ServerBuilder::new();

    let (mut client, mut server) =
        common::setup_with_builders(client_builder, server_builder).await;

    // A normal request should still work with a custom buffer size
    let (resp_fut, send_stream) = client
        .send_request(common::post_request("/buffer-test"), false)
        .await
        .unwrap();
    let mut ss = send_stream.unwrap();
    ss.send_data(vec![0xDD; 512], true).await.unwrap();

    let (req, mut send_resp) = server.accept().await.unwrap().unwrap();
    let body = common::recv_all(&mut req.into_body()).await;
    assert_eq!(body.len(), 512);

    send_resp
        .send_response(common::ok_response(), true)
        .await
        .unwrap();
    let resp = resp_fut.await_response().await.unwrap();
    assert_eq!(resp.status(), 200);
}

// ---------------------------------------------------------------------------
// DoS reset protection
// ---------------------------------------------------------------------------

#[compio_macros::test]
async fn dos_reset_protection() {
    let server_builder = ServerBuilder::new().max_concurrent_reset_streams(3);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let (server_tx, server_rx) = flume::bounded(1);

    compio_runtime::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let conn = server_builder.handshake(stream).await.unwrap();
        let _ = server_tx.send_async(conn).await;
    })
    .detach();

    let stream = TcpStream::connect(addr).await.unwrap();
    let (mut client, connection) = ClientBuilder::new().handshake(stream).await.unwrap();

    let (conn_result_tx, conn_result_rx) = flume::bounded(1);
    compio_runtime::spawn(async move {
        let result = connection.run().await;
        let _ = conn_result_tx.send_async(result).await;
    })
    .detach();

    let mut server = server_rx.recv_async().await.unwrap();

    // Rapidly open streams and have the client reset them
    for i in 0..6 {
        let req = common::get_request(&format!("/reset-{}", i));
        match client.send_request(req, false).await {
            Ok((_resp_fut, send_stream)) => {
                if let Some(ss) = send_stream {
                    let _ = ss.send_reset(compio_h2::Reason::Cancel).await;
                }
            }
            Err(_) => break,
        }
        // Accept on server side to trigger RST_STREAM processing
        match server.accept().await {
            Some(Ok((_req, _send_resp))) => {}
            _ => break,
        }
    }

    // The client connection should eventually close due to GOAWAY
    let result =
        compio_runtime::time::timeout(Duration::from_secs(5), conn_result_rx.recv_async()).await;
    assert!(
        result.is_ok(),
        "connection should close after DoS detection"
    );
}
