//! Flamegraph profiling for HPACK encode/decode hot loop.
//!
//! Purely synchronous — no async runtime needed.
//!
//! Environment variables:
//! - `FLAMEGRAPH_DURATION_SECS` — profiling duration (default: 5)
//! - `FLAMEGRAPH_OUTPUT` — output SVG path (default:
//!   `/tmp/flamegraph-hpack.svg`)
//! - `FLAMEGRAPH_FREQUENCY` — sampling frequency in Hz (default: 10000)

#[cfg(not(unix))]
fn main() {
    eprintln!("flamegraph profiling is only supported on Unix");
}

#[cfg(unix)]
mod support;

#[cfg(unix)]
fn main() {
    use compio_h2::hpack::{Decoder, Encoder};
    let duration_secs: u64 = support::env_or("FLAMEGRAPH_DURATION_SECS", 5);
    let output: String =
        support::env_or("FLAMEGRAPH_OUTPUT", "/tmp/flamegraph-hpack.svg".to_string());

    let mut encoder = Encoder::new(4096);
    let mut decoder = Decoder::new(4096);

    // Mix of static table hits and custom headers
    let headers: Vec<(&[u8], &[u8])> = vec![
        (b":method", b"POST"),
        (b":scheme", b"https"),
        (b":path", b"/grpc.bench.BenchService/Unary"),
        (b":authority", b"localhost:50051"),
        (b"content-type", b"application/grpc"),
        (b"te", b"trailers"),
        (b"user-agent", b"compio-grpc/0.1.0"),
        (b"grpc-encoding", b"identity"),
        (b"x-request-id", b"bench-0000-1111-2222-3333"),
        (
            b"x-custom-header",
            b"some-value-that-is-reasonably-long-for-benchmarking",
        ),
    ];

    let mut buf = Vec::with_capacity(4096);

    // Warm up
    for _ in 0..100 {
        buf.clear();
        encoder.encode(headers.iter().copied(), &mut buf);
        let _ = decoder.decode(&buf).unwrap();
    }

    println!("Starting HPACK flamegraph profiling for {duration_secs}s → {output}");

    let guard = support::profiler_guard();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(duration_secs);
    let mut iterations: u64 = 0;
    let check_interval = 256;

    loop {
        for _ in 0..check_interval {
            buf.clear();
            encoder.encode(headers.iter().copied(), &mut buf);
            let _ = decoder.decode(&buf).unwrap();
            iterations += 1;
        }
        if std::time::Instant::now() >= deadline {
            break;
        }
    }

    println!("Completed {iterations} encode/decode iterations");
    support::write_flamegraph(guard, "HPACK Encode/Decode", &output);
}
