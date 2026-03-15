//! Benchmark for TcpStream write paths — validates the zerocopy threshold.
//!
//! Small writes (< 8KB) use regular send (1 CQE) to avoid the ~40ms
//! delayed-ACK penalty from send_zerocopy's buffer-release CQE.
//! Large writes (>= 8KB) use zerocopy send for kernel copy savings.
//!
//! Run: cargo bench -p compio-net --bench tcp_write

use std::time::Instant;

use compio_buf::{BufResult, IoBuf};
use compio_io::{AsyncRead, AsyncReadExt, AsyncWriteExt, util::Splittable};
use compio_net::{OwnedReadHalf, OwnedWriteHalf, TcpListener, TcpStream};
use criterion::{BenchmarkId, Criterion, Throughput};

fn setup_echo_pair(
    rt: &compio_runtime::Runtime,
) -> (OwnedReadHalf<TcpStream>, OwnedWriteHalf<TcpStream>) {
    rt.block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        compio_runtime::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            stream.set_nodelay(true).unwrap();
            let (mut rh, mut wh) = stream.split();
            loop {
                let buf: Vec<u8> = Vec::with_capacity(65536);
                let BufResult(res, buf) = AsyncRead::read(&mut rh, buf).await;
                match res {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let BufResult(r, _) = wh.write_all(buf).await;
                        if r.is_err() {
                            break;
                        }
                    }
                }
            }
        })
        .detach();

        let stream = TcpStream::connect(addr).await.unwrap();
        stream.set_nodelay(true).unwrap();
        let (rh, wh) = stream.split();
        (rh, wh)
    })
}

fn warmup(
    rt: &compio_runtime::Runtime,
    rh: &mut OwnedReadHalf<TcpStream>,
    wh: &mut OwnedWriteHalf<TcpStream>,
) {
    rt.block_on(async {
        for _ in 0..50 {
            let BufResult(r, _) = wh.write_all(vec![0u8; 100]).await;
            r.unwrap();
            let BufResult(r, _) = rh.read_exact(Vec::with_capacity(100)).await;
            r.unwrap();
        }
    });
}

/// Reusable send+recv buffer pair. After each roundtrip the buffers are
/// returned with their length reset, avoiding per-iteration allocation.
fn roundtrip_buf(size: usize) -> (Vec<u8>, Vec<u8>) {
    (vec![0u8; size], Vec::with_capacity(size))
}

fn bench_write_roundtrip(c: &mut Criterion, rt: &compio_runtime::Runtime) {
    let mut group = c.benchmark_group("tcp_write_roundtrip");

    let (mut rh, mut wh) = setup_echo_pair(rt);
    warmup(rt, &mut rh, &mut wh);

    for &size in &[13, 100, 1024, 8191, 8192, 16384, 65536] {
        let label = match size {
            13 => "13B",
            100 => "100B",
            1024 => "1KB",
            8191 => "8KB-1",
            8192 => "8KB",
            16384 => "16KB",
            65536 => "64KB",
            _ => unreachable!(),
        };
        group.throughput(Throughput::Bytes((size * 2) as u64));
        group.bench_function(BenchmarkId::new("echo", label), |b| {
            b.iter_custom(|iters| {
                rt.block_on(async {
                    let (mut send_buf, mut recv_buf) = roundtrip_buf(size);
                    let start = Instant::now();
                    for _ in 0..iters {
                        let BufResult(r, returned) = wh.write_all(send_buf).await;
                        r.unwrap();
                        send_buf = returned;
                        // Reset length so the buffer can be reused as a
                        // write source on the next iteration.
                        debug_assert_eq!(send_buf.buf_len(), size);

                        let BufResult(r, returned) = rh.read_exact(recv_buf).await;
                        r.unwrap();
                        recv_buf = returned;
                        // SAFETY: read_exact filled the buffer; clear for reuse.
                        unsafe { recv_buf.set_len(0) };
                    }
                    start.elapsed()
                })
            });
        });
    }

    group.finish();
}

fn bench_small_after_large(c: &mut Criterion, rt: &compio_runtime::Runtime) {
    let mut group = c.benchmark_group("tcp_write_small_after_large");

    let (mut rh, mut wh) = setup_echo_pair(rt);
    warmup(rt, &mut rh, &mut wh);

    // The pattern that triggered the 40ms stall: large write then small write.
    // Without the zerocopy threshold fix, the 13B write stalls ~40ms waiting
    // for the delayed-ACK CQE. With the fix, it completes in microseconds.
    group.bench_function("64KB_then_13B", |b| {
        b.iter_custom(|iters| {
            rt.block_on(async {
                let (mut big_send, mut big_recv) = roundtrip_buf(65536);
                let (mut small_send, mut small_recv) = roundtrip_buf(13);
                let start = Instant::now();
                for _ in 0..iters {
                    let BufResult(r, ret) = wh.write_all(big_send).await;
                    r.unwrap();
                    big_send = ret;
                    let BufResult(r, ret) = rh.read_exact(big_recv).await;
                    r.unwrap();
                    big_recv = ret;
                    unsafe { big_recv.set_len(0) };

                    let BufResult(r, ret) = wh.write_all(small_send).await;
                    r.unwrap();
                    small_send = ret;
                    let BufResult(r, ret) = rh.read_exact(small_recv).await;
                    r.unwrap();
                    small_recv = ret;
                    unsafe { small_recv.set_len(0) };
                }
                start.elapsed()
            })
        });
    });

    group.finish();
}

fn bench_write_vectored(c: &mut Criterion, rt: &compio_runtime::Runtime) {
    use compio_io::AsyncWrite;

    let mut group = c.benchmark_group("tcp_write_vectored");

    let (mut rh, mut wh) = setup_echo_pair(rt);
    warmup(rt, &mut rh, &mut wh);

    // Small vectored write: two 7-byte buffers = 14B total (below 8KB threshold).
    // Without the threshold fix, this would use send_zerocopy_vectored and
    // stall ~40ms waiting for the delayed-ACK buffer-release CQE.
    group.throughput(Throughput::Bytes(14 * 2));
    group.bench_function("small_14B", |b| {
        b.iter_custom(|iters| {
            rt.block_on(async {
                let start = Instant::now();
                for _ in 0..iters {
                    let bufs = [vec![0u8; 7], vec![0u8; 7]];
                    let BufResult(r, _) = AsyncWrite::write_vectored(&mut wh, bufs).await;
                    r.unwrap();
                    let mut recv_buf = Vec::with_capacity(14);
                    let BufResult(r, ret) = rh.read_exact(recv_buf).await;
                    r.unwrap();
                    recv_buf = ret;
                    drop(recv_buf);
                }
                start.elapsed()
            })
        });
    });

    // Large vectored write: two 8KB buffers = 16KB total (above threshold).
    group.throughput(Throughput::Bytes(16384 * 2));
    group.bench_function("large_16KB", |b| {
        b.iter_custom(|iters| {
            rt.block_on(async {
                let start = Instant::now();
                for _ in 0..iters {
                    let bufs = [vec![0u8; 8192], vec![0u8; 8192]];
                    let BufResult(r, _) = AsyncWrite::write_vectored(&mut wh, bufs).await;
                    r.unwrap();
                    let mut recv_buf = Vec::with_capacity(16384);
                    let BufResult(r, ret) = rh.read_exact(recv_buf).await;
                    r.unwrap();
                    recv_buf = ret;
                    drop(recv_buf);
                }
                start.elapsed()
            })
        });
    });

    group.finish();
}

fn main() {
    let rt = compio_runtime::Runtime::new().unwrap();
    let mut c = Criterion::default().configure_from_args();

    bench_write_roundtrip(&mut c, &rt);
    bench_small_after_large(&mut c, &rt);
    bench_write_vectored(&mut c, &rt);
}
