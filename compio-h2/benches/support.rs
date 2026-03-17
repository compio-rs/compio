// Shared helpers for compio-h2 benchmarks, flamegraph binaries, and soak tests.
#![allow(dead_code)]

#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::{fs::File, io::Write};

/// Read an environment variable or return a default value.
pub fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Create a pprof profiler guard with configurable frequency.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn profiler_guard() -> pprof::ProfilerGuard<'static> {
    let frequency: i32 = env_or("FLAMEGRAPH_FREQUENCY", 10_000);
    pprof::ProfilerGuardBuilder::default()
        .frequency(frequency)
        .build()
        .expect("failed to build profiler guard")
}

/// Stop the profiler and write a flamegraph SVG to the given path.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn write_flamegraph(guard: pprof::ProfilerGuard, title: &str, output_path: &str) {
    let report = guard.report().build().expect("failed to build report");
    let mut file = File::create(output_path).expect("failed to create flamegraph file");
    let mut opts = pprof::flamegraph::Options::default();
    opts.title = title.to_string();
    report
        .flamegraph_with_options(&mut file, &mut opts)
        .expect("failed to write flamegraph");
    file.flush().expect("failed to flush flamegraph file");
    println!("Flamegraph written to {output_path}");
}

/// Create a deterministic payload of `size` bytes.
pub fn make_payload(size: usize) -> bytes::Bytes {
    bytes::Bytes::from((0..size).map(|i| (i % 256) as u8).collect::<Vec<_>>())
}

/// Read current RSS (Resident Set Size) in KB from `/proc/self/status`.
///
/// Returns `None` on non-Linux or if the file cannot be read.
pub fn get_rss_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if line.starts_with("VmRSS:") {
            return line.split_whitespace().nth(1)?.parse().ok();
        }
    }
    None
}

/// Human-readable label for common benchmark payload sizes.
pub fn size_label(size: usize) -> &'static str {
    match size {
        100 => "100B",
        1024 => "1KB",
        65536 => "64KB",
        _ => panic!("unexpected benchmark payload size: {size}"),
    }
}

// ---------------------------------------------------------------------------
// compio H2 helpers
// ---------------------------------------------------------------------------

/// Echo server loop: accepts H2 connections and echoes request bodies back.
pub async fn compio_h2_server_loop(listener: compio_net::TcpListener) {
    loop {
        let (stream, _) = listener.accept().await.unwrap();
        stream.set_nodelay(true).unwrap();
        compio_runtime::spawn(async move {
            let mut conn = compio_h2::server::builder()
                .initial_window_size(1 << 20)
                .initial_connection_window_size(1 << 20)
                .handshake(stream)
                .await
                .unwrap();
            while let Some(result) = conn.accept().await {
                let (req, mut send_resp) = result.unwrap();
                compio_runtime::spawn(async move {
                    let mut recv = req.into_body();
                    let mut body = Vec::new();
                    while let Some(chunk) = recv.data().await {
                        let data = chunk.unwrap();
                        let len = data.len();
                        body.extend_from_slice(&data);
                        let _ = recv.flow_control().release_capacity(len);
                    }
                    let response = http::Response::builder().status(200).body(()).unwrap();
                    let send_stream = send_resp.send_response(response, false).await.unwrap();
                    let mut send_stream = send_stream.unwrap();
                    send_stream
                        .send_data(bytes::Bytes::from(body), true)
                        .await
                        .unwrap();
                })
                .detach();
            }
        })
        .detach();
    }
}

/// Connect an H2 client and spawn the connection driver.
pub async fn compio_h2_client_connect(
    addr: std::net::SocketAddr,
) -> compio_h2::client::SendRequest {
    let stream = compio_net::TcpStream::connect(addr).await.unwrap();
    stream.set_nodelay(true).unwrap();
    let (send_req, conn) = compio_h2::client::builder()
        .initial_window_size(1 << 20)
        .initial_connection_window_size(1 << 20)
        .handshake(stream)
        .await
        .unwrap();
    compio_runtime::spawn(async move {
        if let Err(e) = conn.run().await {
            eprintln!("client connection error: {e}");
        }
    })
    .detach();
    send_req
}

/// Send an H2 POST request with `body` and drain the response.
pub async fn compio_h2_roundtrip(
    send_req: &mut compio_h2::client::SendRequest,
    body: bytes::Bytes,
) {
    let request = http::Request::builder()
        .method(http::Method::POST)
        .uri("http://localhost/bench")
        .body(())
        .unwrap();
    let (resp_fut, send_stream) = send_req.send_request(request, false).await.unwrap();
    let mut send_stream = send_stream.unwrap();
    send_stream.send_data(body, true).await.unwrap();

    let resp = resp_fut.await_response().await.unwrap();
    let mut recv = resp.into_body();
    while let Some(chunk) = recv.data().await {
        let data = chunk.unwrap();
        let len = data.len();
        let _ = recv.flow_control().release_capacity(len);
    }
}
