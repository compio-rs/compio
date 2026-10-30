//! `h2`, written against `tokio::io`, over `PollFd` sockets and TLS streams.

use std::{
    net::{Ipv4Addr, SocketAddr, TcpListener},
    sync::Arc,
};

use bytes::Bytes;
use compio_runtime::fd::PollFd;
use compio_tls::{TlsAcceptor, TlsConnector};
use rustls::{
    ClientConfig, RootCertStore, ServerConfig,
    crypto::ring::default_provider,
    pki_types::{CertificateDer, PrivateKeyDer},
};
use socket2::{Domain, Protocol, Socket, Type};

/// The response body: larger than the initial flow-control windows (64 KiB)
/// and the read buffers, so it goes in many frames and the client must grant
/// more window as it reads.
fn body() -> Bytes {
    (0..1u32 << 20).map(|i| i as u8).collect()
}

/// Answers each request with [`body`].
async fn serve<T>(stream: T)
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut connection = h2::server::handshake(stream).await.unwrap();
    while let Some(request) = connection.accept().await {
        let (_, mut respond) = request.unwrap();
        let mut send = respond
            .send_response(http::Response::new(()), false)
            .unwrap();
        send.send_data(body(), true).unwrap();
    }
}

/// Sends one request, returns the response body.
async fn ask<T>(stream: T) -> Bytes
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + 'static,
{
    let (mut client, connection) = h2::client::handshake(stream).await.unwrap();
    compio_runtime::spawn(async move { connection.await.unwrap() }).detach();
    let request = http::Request::get("https://localhost/").body(()).unwrap();
    let (response, _) = client.send_request(request, true).unwrap();
    let response = response.await.unwrap();
    assert_eq!(response.status(), http::StatusCode::OK);
    let mut body = response.into_body();
    let mut received = Vec::new();
    while let Some(chunk) = body.data().await {
        let chunk = chunk.unwrap();
        body.flow_control().release_capacity(chunk.len()).unwrap();
        received.extend_from_slice(&chunk);
    }
    received.into()
}

fn listen() -> (PollFd<TcpListener>, SocketAddr) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    (PollFd::new(listener).unwrap(), addr)
}

async fn connect(addr: SocketAddr) -> PollFd<Socket> {
    let stream = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP)).unwrap();
    let stream = PollFd::new(stream).unwrap();
    stream.connect(&addr.into()).await.unwrap();
    stream
}

#[compio_macros::test]
async fn h2_over_tcp() {
    let (listener, addr) = listen();
    compio_runtime::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        serve(stream).await;
    })
    .detach();

    assert_eq!(ask(connect(addr).await).await, body());
}

#[compio_macros::test]
async fn h2_over_tls() {
    let key = rcgen::KeyPair::generate().unwrap();
    let cert = rcgen::CertificateParams::new(["localhost".into()])
        .unwrap()
        .self_signed(&key)
        .unwrap();
    let cert = CertificateDer::from(cert.der().to_vec());
    let key = PrivateKeyDer::try_from(key.serialize_der()).unwrap();
    let provider = Arc::new(default_provider());

    let mut server = ServerConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert.clone()], key)
        .unwrap();
    server.alpn_protocols = vec![b"h2".to_vec()];
    let acceptor = TlsAcceptor::from(Arc::new(server));

    let mut roots = RootCertStore::empty();
    roots.add(cert).unwrap();
    let mut client = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    client.alpn_protocols = vec![b"h2".to_vec()];
    let connector = TlsConnector::from(Arc::new(client));

    let (listener, addr) = listen();
    compio_runtime::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        serve(acceptor.accept(stream).await.unwrap()).await;
    })
    .detach();

    let stream = connector
        .connect("localhost", connect(addr).await)
        .await
        .unwrap();
    assert_eq!(stream.negotiated_alpn().as_deref(), Some(&b"h2"[..]));
    assert_eq!(ask(stream).await, body());
}
