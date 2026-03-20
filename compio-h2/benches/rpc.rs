use std::time::Instant;

use bytes::Bytes;
use criterion::{BenchmarkId, Criterion, Throughput};

mod support;
use support::{
    compio_h2_client_connect, compio_h2_roundtrip, compio_h2_server_loop, make_payload, size_label,
};

// ---------------------------------------------------------------------------
// tokio / h2 crate helpers
// ---------------------------------------------------------------------------

async fn h2_crate_server_loop(listener: tokio::net::TcpListener) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            return;
        };
        let _ = stream.set_nodelay(true);
        tokio::spawn(async move {
            let Ok(mut conn) = h2::server::Builder::new()
                .initial_window_size(1 << 20)
                .initial_connection_window_size(1 << 20)
                .handshake(stream)
                .await
            else {
                return;
            };
            while let Some(Ok((request, mut respond))) = conn.accept().await {
                tokio::spawn(async move {
                    let mut body = request.into_body();
                    let mut data = Vec::new();
                    while let Some(Ok(chunk)) = body.data().await {
                        let _ = body.flow_control().release_capacity(chunk.len());
                        data.extend_from_slice(&chunk);
                    }
                    let response = http::Response::builder().status(200).body(()).unwrap();
                    let Ok(mut send_stream) = respond.send_response(response, false) else {
                        return;
                    };
                    let _ = send_stream.send_data(Bytes::from(data), true);
                });
            }
        });
    }
}

async fn h2_crate_client_connect(addr: std::net::SocketAddr) -> h2::client::SendRequest<Bytes> {
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    stream.set_nodelay(true).unwrap();
    let (send_req, conn) = h2::client::Builder::new()
        .initial_window_size(1 << 20)
        .initial_connection_window_size(1 << 20)
        .handshake(stream)
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = conn.await;
    });
    send_req
}

async fn h2_crate_roundtrip(send_req: &mut h2::client::SendRequest<Bytes>, body: Bytes) {
    let request = http::Request::builder()
        .method(http::Method::POST)
        .uri("http://localhost/bench")
        .body(())
        .unwrap();
    let (resp_fut, mut send_stream) = send_req.send_request(request, false).unwrap();
    send_stream.send_data(body, true).unwrap();

    let resp = resp_fut.await.unwrap();
    let mut recv = resp.into_body();
    while let Some(chunk) = recv.data().await {
        let chunk = chunk.unwrap();
        let _ = recv.flow_control().release_capacity(chunk.len());
    }
}

// ---------------------------------------------------------------------------
// H2 Handshake benchmark
// ---------------------------------------------------------------------------

fn bench_h2_handshake(
    c: &mut Criterion,
    compio_rt: &compio_runtime::Runtime,
    tokio_rt: &tokio::runtime::Runtime,
) {
    let mut group = c.benchmark_group("h2_handshake");

    // Spawn persistent compio server
    let compio_addr = compio_rt.block_on(async {
        let listener = compio_net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        compio_runtime::spawn(compio_h2_server_loop(listener)).detach();
        addr
    });

    // Spawn persistent tokio server
    let tokio_addr = tokio_rt.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(h2_crate_server_loop(listener));
        addr
    });

    group.bench_function("compio", |b| {
        b.to_async(compio_rt).iter(|| async {
            let stream = compio_net::TcpStream::connect(compio_addr).await.unwrap();
            let (_send_req, conn) = compio_h2::client::handshake(stream).await.unwrap();
            // Don't detach — dropping the Task cancels conn.run() and closes
            // the socket, preventing ephemeral port exhaustion.
            let _handle = compio_runtime::spawn(async move {
                let _ = conn.run().await;
            });
        });
    });

    group.bench_function("h2_crate", |b| {
        b.to_async(tokio_rt).iter(|| async {
            let stream = tokio::net::TcpStream::connect(tokio_addr).await.unwrap();
            let (_send_req, conn) = h2::client::handshake(stream).await.unwrap();
            let handle = tokio::spawn(async move {
                let _ = conn.await;
            });
            handle.abort();
        });
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// H2 Request/Response benchmark
// ---------------------------------------------------------------------------

fn bench_h2_request(
    c: &mut Criterion,
    compio_rt: &compio_runtime::Runtime,
    tokio_rt: &tokio::runtime::Runtime,
) {
    let mut group = c.benchmark_group("h2_request");

    // Spawn persistent compio server + client
    let mut compio_send_req = compio_rt.block_on(async {
        let listener = compio_net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        compio_runtime::spawn(compio_h2_server_loop(listener)).detach();
        compio_h2_client_connect(addr).await
    });
    // Warm up compio connection
    compio_rt.block_on(async {
        for _ in 0..5 {
            compio_h2_roundtrip(&mut compio_send_req, make_payload(100)).await;
        }
    });

    // Spawn persistent tokio server + client
    let mut tokio_send_req = tokio_rt.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(h2_crate_server_loop(listener));
        h2_crate_client_connect(addr).await
    });
    // Warm up tokio connection
    tokio_rt.block_on(async {
        for _ in 0..5 {
            h2_crate_roundtrip(&mut tokio_send_req, make_payload(100)).await;
        }
    });

    for &size in &[100, 1024, 65536] {
        let label = size_label(size);
        let payload = make_payload(size);
        // Throughput = payload sent + echoed back
        group.throughput(Throughput::Bytes((size * 2) as u64));

        group.bench_function(BenchmarkId::new("compio", label), |b| {
            let body = payload.clone();
            b.iter_custom(|iters| {
                let body = body.clone();
                compio_rt.block_on(async {
                    let start = Instant::now();
                    for _ in 0..iters {
                        compio_h2_roundtrip(&mut compio_send_req, body.clone()).await;
                    }
                    start.elapsed()
                })
            });
        });

        group.bench_function(BenchmarkId::new("h2_crate", label), |b| {
            let body = payload.clone();
            b.iter_custom(|iters| {
                let body = body.clone();
                tokio_rt.block_on(async {
                    let start = Instant::now();
                    for _ in 0..iters {
                        h2_crate_roundtrip(&mut tokio_send_req, body.clone()).await;
                    }
                    start.elapsed()
                })
            });
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Manual main — persistent runtimes, reused across all benchmarks
// ---------------------------------------------------------------------------

fn main() {
    let compio_rt = compio_runtime::Runtime::new().unwrap();
    let tokio_rt = tokio::runtime::Runtime::new().unwrap();

    let mut c = Criterion::default().configure_from_args();

    bench_h2_handshake(&mut c, &compio_rt, &tokio_rt);
    bench_h2_request(&mut c, &compio_rt, &tokio_rt);
}
