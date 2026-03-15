//! Tests for the zerocopy send threshold in TcpStream::write.
//!
//! Small writes (< 8KB) use regular send (1 CQE) to avoid the ~40ms
//! delayed-ACK penalty from the zerocopy buffer-release CQE.
//! Large writes (>= 8KB) use zerocopy send for kernel copy savings.

use compio_buf::BufResult;
use compio_io::{AsyncReadExt, AsyncWriteExt};
use compio_net::{TcpListener, TcpStream};

async fn echo_pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = compio_runtime::spawn(async move { listener.accept().await.unwrap() });
    let client = TcpStream::connect(&addr).await.unwrap();
    let (server, _) = task.await.unwrap();
    (client, server)
}

/// Small write (below threshold) completes and delivers data correctly.
#[compio_macros::test]
async fn small_write_delivers_data() {
    let (mut tx, mut rx) = echo_pair().await;

    let payload = vec![0xABu8; 13]; // 13 bytes — WINDOW_UPDATE size
    let BufResult(res, _) = tx.write_all(payload.clone()).await;
    res.unwrap();

    let BufResult(res, buf) = rx.read_exact(Vec::with_capacity(13)).await;
    res.unwrap();
    assert_eq!(buf, payload);
}

/// Large write (above threshold) completes and delivers data correctly.
#[compio_macros::test]
async fn large_write_delivers_data() {
    let (mut tx, mut rx) = echo_pair().await;

    let payload = vec![0xCDu8; 16 * 1024]; // 16KB — above 8KB threshold
    let BufResult(res, _) = tx.write_all(payload.clone()).await;
    res.unwrap();

    let BufResult(res, buf) = rx.read_exact(Vec::with_capacity(16 * 1024)).await;
    res.unwrap();
    assert_eq!(buf, payload);
}

/// Write exactly at threshold boundary.
#[compio_macros::test]
async fn write_at_threshold_boundary() {
    let (mut tx, mut rx) = echo_pair().await;

    // Exactly 8KB — at threshold, should use zerocopy
    let payload = vec![0xEEu8; 8 * 1024];
    let BufResult(res, _) = tx.write_all(payload.clone()).await;
    res.unwrap();

    let BufResult(res, buf) = rx.read_exact(Vec::with_capacity(8 * 1024)).await;
    res.unwrap();
    assert_eq!(buf, payload);
}

/// Write just below threshold boundary.
#[compio_macros::test]
async fn write_below_threshold_boundary() {
    let (mut tx, mut rx) = echo_pair().await;

    // 8KB - 1 — just below threshold, should use regular send
    let payload = vec![0xFFu8; 8 * 1024 - 1];
    let BufResult(res, _) = tx.write_all(payload.clone()).await;
    res.unwrap();

    let BufResult(res, buf) = rx.read_exact(Vec::with_capacity(8 * 1024 - 1)).await;
    res.unwrap();
    assert_eq!(buf, payload);
}

/// Multiple small writes in sequence all deliver correctly.
#[compio_macros::test]
async fn sequential_small_writes() {
    let (mut tx, mut rx) = echo_pair().await;

    for i in 0u8..20 {
        let payload = vec![i; 13];
        let BufResult(res, _) = tx.write_all(payload).await;
        res.unwrap();
    }

    for i in 0u8..20 {
        let BufResult(res, buf) = rx.read_exact(Vec::with_capacity(13)).await;
        res.unwrap();
        assert_eq!(buf, vec![i; 13]);
    }
}

/// Mixed small and large writes interleaved.
#[compio_macros::test]
async fn mixed_small_large_writes() {
    let (mut tx, mut rx) = echo_pair().await;

    // Small
    let BufResult(res, _) = tx.write_all(vec![1u8; 13]).await;
    res.unwrap();
    // Large
    let BufResult(res, _) = tx.write_all(vec![2u8; 16 * 1024]).await;
    res.unwrap();
    // Small
    let BufResult(res, _) = tx.write_all(vec![3u8; 100]).await;
    res.unwrap();

    let BufResult(res, buf) = rx.read_exact(Vec::with_capacity(13)).await;
    res.unwrap();
    assert_eq!(buf, vec![1u8; 13]);

    let BufResult(res, buf) = rx.read_exact(Vec::with_capacity(16 * 1024)).await;
    res.unwrap();
    assert_eq!(buf, vec![2u8; 16 * 1024]);

    let BufResult(res, buf) = rx.read_exact(Vec::with_capacity(100)).await;
    res.unwrap();
    assert_eq!(buf, vec![3u8; 100]);
}

/// Small vectored write (below threshold) uses regular send.
#[compio_macros::test]
async fn small_vectored_write() {
    use compio_io::AsyncWrite;

    let (mut tx, mut rx) = echo_pair().await;

    // Two small buffers totaling 14 bytes — below 8KB threshold
    let bufs = [vec![0xAAu8; 7], vec![0xBBu8; 7]];
    let BufResult(res, _) = AsyncWrite::write_vectored(&mut tx, bufs).await;
    let n = res.unwrap();
    assert_eq!(n, 14);

    let BufResult(res, buf) = rx.read_exact(Vec::with_capacity(14)).await;
    res.unwrap();
    assert_eq!(&buf[..7], &[0xAAu8; 7]);
    assert_eq!(&buf[7..], &[0xBBu8; 7]);
}

/// Large vectored write (above threshold) uses zerocopy.
#[compio_macros::test]
async fn large_vectored_write() {
    use compio_io::AsyncWrite;

    let (mut tx, mut rx) = echo_pair().await;

    // Two 8KB buffers = 16KB total — above threshold
    let bufs = [vec![0xCCu8; 8192], vec![0xDDu8; 8192]];
    let BufResult(res, _) = AsyncWrite::write_vectored(&mut tx, bufs).await;
    let n = res.unwrap();
    assert_eq!(n, 16384);

    let BufResult(res, buf) = rx.read_exact(Vec::with_capacity(16384)).await;
    res.unwrap();
    assert_eq!(&buf[..8192], &[0xCCu8; 8192]);
    assert_eq!(&buf[8192..], &[0xDDu8; 8192]);
}

/// Small write with split halves (OwnedWriteHalf).
#[compio_macros::test]
async fn small_write_split_halves() {
    use compio_io::util::Splittable;

    let (tx, rx) = echo_pair().await;
    let (_rtx, mut wtx) = Splittable::split(tx);
    let (mut rrx, _wrx) = Splittable::split(rx);

    let payload = vec![0xABu8; 13];
    let BufResult(res, _) = wtx.write_all(payload.clone()).await;
    res.unwrap();

    let BufResult(res, buf) = rrx.read_exact(Vec::with_capacity(13)).await;
    res.unwrap();
    assert_eq!(buf, payload);
}
