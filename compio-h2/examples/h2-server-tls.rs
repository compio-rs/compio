/// HTTP/2 TLS server example.
///
/// Usage: h2-server-tls [PORT]
///        H2_PORT=9443 h2-server-tls
///
/// Environment variables:
///   H2_PORT                   — listen port (default 8443)
///   H2_IDENTITY_PATH          — path to PKCS#12 identity file (.p12)
///   H2_IDENTITY_PASS          — password for the PKCS#12 file (default: empty)
///   H2_MAX_CONCURRENT_STREAMS — max concurrent streams (default 100)
///   H2_INITIAL_WINDOW_SIZE    — initial window size in bytes (default 65535)
///
/// GET requests return a static body. POST requests echo the request body.
use std::env;
use std::{
    future::poll_fn,
    sync::atomic::{AtomicU64, Ordering},
    task::Poll,
};

use bytes::{Bytes, BytesMut};
use compio_h2::server;
use compio_h2::tls::{self, TlsAcceptor};
use compio_net::TcpListener;

static REQUEST_COUNT: AtomicU64 = AtomicU64::new(0);

/// Yield control to the executor once, allowing other tasks to run.
async fn yield_now() {
    let mut yielded = false;
    poll_fn(|cx| {
        if yielded {
            Poll::Ready(())
        } else {
            yielded = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    })
    .await;
}

/// Read the full body from a RecvStream into a single Bytes.
async fn collect_body(mut recv: compio_h2::RecvStream) -> Result<Bytes, compio_h2::H2Error> {
    let mut buf = BytesMut::new();
    while let Some(result) = recv.data().await {
        buf.extend_from_slice(&result?);
    }
    Ok(buf.freeze())
}

fn env_u32(name: &str, default: u32) -> u32 {
    env::var(name)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

fn build_acceptor() -> TlsAcceptor {
    // Load a PKCS#12 identity from environment variables.
    //
    // Generate a self-signed one for local testing:
    //   openssl req -x509 -newkey rsa:2048 -keyout key.pem -out cert.pem \
    //     -days 365 -nodes -subj '/CN=localhost'
    //   openssl pkcs12 -export -out identity.p12 -inkey key.pem -in cert.pem \
    //     -passout pass:changeit
    //
    //   H2_IDENTITY_PATH=identity.p12 H2_IDENTITY_PASS=changeit h2-server-tls
    let path = env::var("H2_IDENTITY_PATH").expect(
        "set H2_IDENTITY_PATH to a PKCS#12 file (see example header for generation commands)",
    );
    let pass = env::var("H2_IDENTITY_PASS").unwrap_or_default();
    let pkcs12 = std::fs::read(&path).expect("failed to read identity file");
    let identity = compio_h2::tls::native_tls::Identity::from_pkcs12(&pkcs12, &pass)
        .expect("failed to parse PKCS#12 identity");
    let native_acceptor = compio_h2::tls::native_tls::TlsAcceptor::builder(identity)
        .build()
        .unwrap();
    TlsAcceptor::from(native_acceptor)
}

#[compio_macros::main]
async fn main() {
    let port: u16 = env::args()
        .nth(1)
        .or_else(|| env::var("H2_PORT").ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(8443);

    let max_concurrent = env_u32("H2_MAX_CONCURRENT_STREAMS", 100);
    let initial_window = env_u32("H2_INITIAL_WINDOW_SIZE", 65535);

    let acceptor = build_acceptor();
    let listener = TcpListener::bind(("0.0.0.0", port)).await.unwrap();
    eprintln!("h2-server-tls listening on 0.0.0.0:{port}");
    eprintln!("  max_concurrent_streams={max_concurrent} initial_window_size={initial_window}");

    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => {
                eprintln!("accept error: {e}");
                continue;
            }
        };

        let acceptor = acceptor.clone();
        compio_runtime::spawn(async move {
            let tls_stream = match tls::accept(&acceptor, stream).await {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[{peer}] TLS accept error: {e}");
                    return;
                }
            };

            let mut conn = match server::builder()
                .max_concurrent_streams(max_concurrent)
                .initial_window_size(initial_window)
                .handshake(tls_stream)
                .await
            {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("[{peer}] handshake error: {e}");
                    return;
                }
            };

            while let Some(result) = conn.accept().await {
                let (req, mut send_resp) = match result {
                    Ok(pair) => pair,
                    Err(e) => {
                        eprintln!("[{peer}] accept stream error: {e}");
                        return;
                    }
                };

                let count = REQUEST_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
                if count.is_multiple_of(100) {
                    eprintln!("[stats] {count} requests served");
                }

                let (parts, recv_body) = req.into_parts();
                let is_post = parts.method == http::Method::POST;

                yield_now().await;

                let body = if is_post {
                    match collect_body(recv_body).await {
                        Ok(b) => b,
                        Err(e) => {
                            eprintln!("[{peer}] read body error: {e}");
                            continue;
                        }
                    }
                } else {
                    Bytes::from_static(b"hello h2 over tls\n")
                };

                let response = http::Response::builder()
                    .status(200)
                    .header("content-type", "application/octet-stream")
                    .body(())
                    .unwrap();

                match send_resp.send_response(response, false).await {
                    Ok(Some(mut stream)) => {
                        if let Err(e) = stream.send_data(body, true).await {
                            eprintln!("[{peer}] send body error: {e}");
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        eprintln!("[{peer}] send response error: {e}");
                    }
                }
            }
        })
        .detach();
    }
}
