//! Flamegraph profiling for H2 request/response over TCP.
//!
//! Starts a compio H2 server, connects a client, and runs a hot loop of
//! request/response cycles.
//!
//! Environment variables:
//! - `FLAMEGRAPH_DURATION_SECS` — profiling duration (default: 5)
//! - `FLAMEGRAPH_OUTPUT` — output SVG path (default:
//!   `/tmp/flamegraph-h2-request.svg`)
//! - `FLAMEGRAPH_FREQUENCY` — sampling frequency in Hz (default: 10000)

#[cfg(not(unix))]
fn main() {
    eprintln!("flamegraph profiling is only supported on Unix");
}

#[cfg(unix)]
mod support;

#[cfg(unix)]
fn main() {
    use bytes::Bytes;
    use compio_net::TcpListener;
    let duration_secs: u64 = support::env_or("FLAMEGRAPH_DURATION_SECS", 5);
    let output: String = support::env_or(
        "FLAMEGRAPH_OUTPUT",
        "/tmp/flamegraph-h2-request.svg".to_string(),
    );

    compio_runtime::Runtime::new().unwrap().block_on(async {
        // Bind server
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        // Spawn server
        compio_runtime::spawn(support::compio_h2_server_loop(listener)).detach();

        // Connect client and do H2 handshake
        let mut send_req = support::compio_h2_client_connect(addr).await;

        let body = Bytes::from_static(b"hello benchmark");

        // Warm up
        for _ in 0..100 {
            support::compio_h2_roundtrip(&mut send_req, body.clone()).await;
        }

        println!("Starting H2 request flamegraph profiling for {duration_secs}s → {output}");

        let guard = support::profiler_guard();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(duration_secs);
        let mut iterations: u64 = 0;
        let check_interval = 64;

        loop {
            for _ in 0..check_interval {
                support::compio_h2_roundtrip(&mut send_req, body.clone()).await;
                iterations += 1;
            }
            if std::time::Instant::now() >= deadline {
                break;
            }
        }

        println!("Completed {iterations} H2 request/response iterations");
        support::write_flamegraph(guard, "H2 Request/Response", &output);
    });
}
