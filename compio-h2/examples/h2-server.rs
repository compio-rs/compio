/// HTTP/2 server example.
///
/// Usage: h2-server [PORT]
///        H2_PORT=9090 h2-server
///
/// Environment variables:
///   H2_PORT                   — listen port (default 8080)
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

#[compio_macros::main]
async fn main() {
    let port: u16 = env::args()
        .nth(1)
        .or_else(|| env::var("H2_PORT").ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(8080);

    let max_concurrent = env_u32("H2_MAX_CONCURRENT_STREAMS", 100);
    let initial_window = env_u32("H2_INITIAL_WINDOW_SIZE", 65535);

    let listener = TcpListener::bind(("0.0.0.0", port)).await.unwrap();
    eprintln!("h2-server listening on 0.0.0.0:{port}");
    eprintln!("  max_concurrent_streams={max_concurrent} initial_window_size={initial_window}");

    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => {
                eprintln!("accept error: {e}");
                continue;
            }
        };

        compio_runtime::spawn(async move {
            let mut conn = match server::builder()
                .max_concurrent_streams(max_concurrent)
                .initial_window_size(initial_window)
                .handshake(stream)
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
                    Bytes::from_static(b"hello h2\n")
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
