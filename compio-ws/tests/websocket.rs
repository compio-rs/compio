#![cfg(feature = "connect")]

use compio_net::{TcpListener, TcpStream};
use compio_ws::{accept_async, client_async};
use futures_channel::oneshot;
use tungstenite::Message;

#[compio_macros::test]
async fn test_handshake() {
    let (tx, rx) = oneshot::channel();

    compio_runtime::spawn(async move {
        let listener = TcpListener::bind("127.0.0.1:12345").await.unwrap();
        tx.send(()).unwrap();

        let (stream, _) = listener.accept().await.unwrap();
        let _ws = accept_async(stream).await.expect("Server handshake failed");
    })
    .detach();

    rx.await.expect("Failed to wait for server");

    let tcp = TcpStream::connect("127.0.0.1:12345")
        .await
        .expect("Failed to connect");
    let (_ws, _response) = client_async("ws://127.0.0.1:12345", tcp)
        .await
        .expect("Client handshake failed");
}

#[compio_macros::test]
async fn test_echo_message() {
    let (tx, rx) = oneshot::channel();

    compio_runtime::spawn(async move {
        let listener = TcpListener::bind("127.0.0.1:12346").await.unwrap();
        tx.send(()).unwrap();

        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();

        let msg = ws.read().await.unwrap();
        ws.send(msg).await.unwrap();
    })
    .detach();

    rx.await.unwrap();

    let tcp = TcpStream::connect("127.0.0.1:12346").await.unwrap();
    let (mut ws, _) = client_async("ws://127.0.0.1:12346", tcp).await.unwrap();

    let test_msg = "Hello, WebSocket!";
    ws.send(Message::Text(test_msg.into())).await.unwrap();

    let response = ws.read().await.unwrap();
    assert_eq!(response, Message::Text(test_msg.into()));
}

#[compio_macros::test]
async fn test_binary_message() {
    let (tx, rx) = oneshot::channel();

    compio_runtime::spawn(async move {
        let listener = TcpListener::bind("127.0.0.1:12347").await.unwrap();
        tx.send(()).unwrap();

        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();

        let msg = ws.read().await.unwrap();
        ws.send(msg).await.unwrap();
    })
    .detach();

    rx.await.unwrap();

    let tcp = TcpStream::connect("127.0.0.1:12347").await.unwrap();
    let (mut ws, _) = client_async("ws://127.0.0.1:12347", tcp).await.unwrap();

    let test_data = vec![1, 2, 3, 4, 5];
    ws.send(Message::Binary(test_data.clone().into()))
        .await
        .unwrap();

    let response = ws.read().await.unwrap();
    assert_eq!(response, Message::Binary(test_data.into()));
}

#[compio_macros::test]
async fn test_ping_pong() {
    let (tx, rx) = oneshot::channel();

    compio_runtime::spawn(async move {
        let listener = TcpListener::bind("127.0.0.1:12348").await.unwrap();
        tx.send(()).unwrap();

        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();

        let msg = ws.read().await.unwrap();
        if let Message::Ping(data) = msg {
            ws.send(Message::Pong(data)).await.unwrap();
        }
    })
    .detach();

    rx.await.unwrap();

    let tcp = TcpStream::connect("127.0.0.1:12348").await.unwrap();
    let (mut ws, _) = client_async("ws://127.0.0.1:12348", tcp).await.unwrap();

    let ping_data = vec![42];
    ws.send(Message::Ping(ping_data.clone().into()))
        .await
        .unwrap();

    let response = ws.read().await.unwrap();
    assert_eq!(response, Message::Pong(ping_data.into()));
}

#[compio_macros::test]
async fn test_close_handshake() {
    let (tx, rx) = oneshot::channel();

    compio_runtime::spawn(async move {
        let listener = TcpListener::bind("127.0.0.1:12349").await.unwrap();
        tx.send(()).unwrap();

        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();

        let msg = ws.read().await.unwrap();
        assert!(msg.is_close());
    })
    .detach();

    rx.await.unwrap();

    let tcp = TcpStream::connect("127.0.0.1:12349").await.unwrap();
    let (mut ws, _) = client_async("ws://127.0.0.1:12349", tcp).await.unwrap();

    ws.close(None).await.unwrap();
}

#[compio_macros::test]
async fn test_multiple_messages() {
    let (tx, rx) = oneshot::channel();

    compio_runtime::spawn(async move {
        let listener = TcpListener::bind("127.0.0.1:12350").await.unwrap();
        tx.send(()).unwrap();

        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = accept_async(stream).await.unwrap();

        for _ in 0..3 {
            let msg = ws.read().await.unwrap();
            ws.send(msg).await.unwrap();
        }
    })
    .detach();

    rx.await.unwrap();

    let tcp = TcpStream::connect("127.0.0.1:12350").await.unwrap();
    let (mut ws, _) = client_async("ws://127.0.0.1:12350", tcp).await.unwrap();

    for i in 0..3 {
        let msg = format!("Message {}", i);
        ws.send(Message::Text(msg.clone().into())).await.unwrap();
        let response = ws.read().await.unwrap();
        assert_eq!(response, Message::Text(msg.into()));
    }
}
