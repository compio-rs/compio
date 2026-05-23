use std::time::Duration;

use compio::io::{AsyncRead, AsyncWrite};
use compio::net::{TcpListener, TcpStream, UnixListener, UnixStream};
use compio_runtime::ResumeUnwind;

#[test]
fn tcp_roundtrip() {
    let rt = compio_runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = compio_runtime::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let buf = vec![0u8; 5];
            let compio::BufResult(res, buf) = AsyncRead::read(&mut &stream, buf).await;
            let n = res.unwrap();
            assert_eq!(&buf[..n], b"hello");
        });

        let client = TcpStream::connect(addr).await.unwrap();
        let compio::BufResult(res, _) = AsyncWrite::write(&mut &client, b"hello".to_vec()).await;
        res.unwrap();

        compio_runtime::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("timed out")
            .resume_unwind()
            .unwrap();
    });
}

#[test]
fn ipc_roundtrip() {
    let rt = compio_runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let path = std::env::temp_dir().join(format!(
            "compio-kq-{}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);

        let listener = UnixListener::bind(&path).await.unwrap();

        let server = compio_runtime::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let buf = vec![0u8; 5];
            let compio::BufResult(res, buf) = AsyncRead::read(&mut &stream, buf).await;
            let n = res.unwrap();
            assert_eq!(&buf[..n], b"hello");
        });

        let client = UnixStream::connect(&path).await.unwrap();
        let compio::BufResult(res, _) = AsyncWrite::write(&mut &client, b"hello".to_vec()).await;
        res.unwrap();

        compio_runtime::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("timed out")
            .resume_unwind()
            .unwrap();

        let _ = std::fs::remove_file(&path);
    });
}

#[test]
fn tcp_roundtrip_repeated() {
    for i in 0..20 {
        let rt = compio_runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();

            let server = compio_runtime::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let buf = vec![0u8; 16];
                let compio::BufResult(res, buf) = AsyncRead::read(&mut &stream, buf).await;
                let n = res.unwrap();
                String::from_utf8(buf[..n].to_vec()).unwrap()
            });

            let client = TcpStream::connect(addr).await.unwrap();
            let msg = format!("msg-{i}");
            let compio::BufResult(res, _) = AsyncWrite::write(&mut &client, msg.as_bytes().to_vec()).await;
            res.unwrap();

            let got = compio_runtime::time::timeout(Duration::from_secs(2), server)
                .await
                .unwrap_or_else(|_| panic!("iteration {i} timed out"))
                .resume_unwind()
                .unwrap();
            assert_eq!(got, msg);
        });
    }
}

// ---------------------------------------------------------------------------
// Multi-round interleaved I/O (ZMTP handshake pattern)
//
// Two spawned tasks on one single-threaded runtime, connected via TCP or IPC.
// Both do write-then-read for multiple rounds. This models the ZMTP 3.x NULL
// handshake: greeting exchange (64B), READY command exchange (~32B), then
// message exchange. The per-round fd interest re-registration across poll
// cycles is the suspected cause of lost events on macOS kqueue.
// ---------------------------------------------------------------------------

async fn tcp_read_exact(stream: &TcpStream, n: usize) -> Vec<u8> {
    let mut out = vec![0u8; n];
    let mut pos = 0;
    while pos < n {
        let buf = vec![0u8; n - pos];
        let compio::BufResult(res, buf) = AsyncRead::read(&mut &*stream, buf).await;
        let got = res.expect("read failed");
        assert!(got > 0, "unexpected EOF after {pos}/{n} bytes");
        out[pos..pos + got].copy_from_slice(&buf[..got]);
        pos += got;
    }
    out
}

async fn tcp_write_all(stream: &TcpStream, data: Vec<u8>) {
    let n = data.len();
    let compio::BufResult(res, _) = AsyncWrite::write(&mut &*stream, data).await;
    assert_eq!(res.expect("write failed"), n);
}

async fn ipc_read_exact(stream: &UnixStream, n: usize) -> Vec<u8> {
    let mut out = vec![0u8; n];
    let mut pos = 0;
    while pos < n {
        let buf = vec![0u8; n - pos];
        let compio::BufResult(res, buf) = AsyncRead::read(&mut &*stream, buf).await;
        let got = res.expect("read failed");
        assert!(got > 0, "unexpected EOF after {pos}/{n} bytes");
        out[pos..pos + got].copy_from_slice(&buf[..got]);
        pos += got;
    }
    out
}

async fn ipc_write_all(stream: &UnixStream, data: Vec<u8>) {
    let n = data.len();
    let compio::BufResult(res, _) = AsyncWrite::write(&mut &*stream, data).await;
    assert_eq!(res.expect("write failed"), n);
}

fn make_payload(tag: u8, round: u32, size: usize) -> Vec<u8> {
    let mut buf = vec![0u8; size];
    buf[0] = tag;
    let rb = round.to_be_bytes();
    let n = rb.len().min(size - 1);
    buf[1..1 + n].copy_from_slice(&rb[..n]);
    buf
}

async fn tcp_multi_round(stream: &TcpStream, my_tag: u8, peer_tag: u8, sizes: &[usize]) {
    for (round, &size) in sizes.iter().enumerate() {
        tcp_write_all(stream, make_payload(my_tag, round as u32, size)).await;
        let got = tcp_read_exact(stream, size).await;
        assert_eq!(
            got[0], peer_tag,
            "round {round}: expected tag {}, got {}",
            peer_tag as char, got[0] as char
        );
    }
}

async fn ipc_multi_round(stream: &UnixStream, my_tag: u8, peer_tag: u8, sizes: &[usize]) {
    for (round, &size) in sizes.iter().enumerate() {
        ipc_write_all(stream, make_payload(my_tag, round as u32, size)).await;
        let got = ipc_read_exact(stream, size).await;
        assert_eq!(
            got[0], peer_tag,
            "round {round}: expected tag {}, got {}",
            peer_tag as char, got[0] as char
        );
    }
}

/// 5 rounds of write-then-read, 64-byte payloads. Two spawned tasks.
#[test]
fn tcp_interleaved_5_rounds() {
    let rt = compio_runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let sizes: &[usize] = &[64; 5];

        let server = compio_runtime::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            tcp_multi_round(&stream, b'S', b'C', sizes).await;
        });

        let client = compio_runtime::spawn(async move {
            let stream = TcpStream::connect(addr).await.unwrap();
            tcp_multi_round(&stream, b'C', b'S', sizes).await;
        });

        compio_runtime::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("server timed out")
            .resume_unwind()
            .unwrap();

        compio_runtime::time::timeout(Duration::from_secs(2), client)
            .await
            .expect("client timed out")
            .resume_unwind()
            .unwrap();
    });
}

/// ZMTP-like pattern: 3 rounds with varying sizes (greeting 64B, handshake
/// 32B, message 8B).
#[test]
fn tcp_interleaved_zmtp_null() {
    let rt = compio_runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let sizes: &[usize] = &[64, 32, 8];

        let server = compio_runtime::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            tcp_multi_round(&stream, b'S', b'C', sizes).await;
        });

        let client = compio_runtime::spawn(async move {
            let stream = TcpStream::connect(addr).await.unwrap();
            tcp_multi_round(&stream, b'C', b'S', sizes).await;
        });

        compio_runtime::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("server timed out")
            .resume_unwind()
            .unwrap();

        compio_runtime::time::timeout(Duration::from_secs(2), client)
            .await
            .expect("client timed out")
            .resume_unwind()
            .unwrap();
    });
}

/// Repeated multi-round to catch intermittent event loss.
#[test]
fn tcp_interleaved_repeated() {
    for i in 0..10 {
        let rt = compio_runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let sizes: &[usize] = &[64; 5];

            let server = compio_runtime::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                tcp_multi_round(&stream, b'S', b'C', sizes).await;
            });

            let client = compio_runtime::spawn(async move {
                let stream = TcpStream::connect(addr).await.unwrap();
                tcp_multi_round(&stream, b'C', b'S', sizes).await;
            });

            compio_runtime::time::timeout(Duration::from_secs(2), server)
                .await
                .unwrap_or_else(|_| panic!("iteration {i}: server timed out"))
                .resume_unwind()
                .unwrap();

            compio_runtime::time::timeout(Duration::from_secs(2), client)
                .await
                .unwrap_or_else(|_| panic!("iteration {i}: client timed out"))
                .resume_unwind()
                .unwrap();
        });
    }
}

/// IPC variant of the 5-round interleaved test.
#[test]
fn ipc_interleaved_5_rounds() {
    let rt = compio_runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let path = std::env::temp_dir().join(format!(
            "compio-kq-interleaved-{}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).await.unwrap();
        let sizes: &[usize] = &[64; 5];
        let path2 = path.clone();

        let server = compio_runtime::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            ipc_multi_round(&stream, b'S', b'C', sizes).await;
        });

        let client = compio_runtime::spawn(async move {
            let stream = UnixStream::connect(&path2).await.unwrap();
            ipc_multi_round(&stream, b'C', b'S', sizes).await;
        });

        compio_runtime::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("server timed out")
            .resume_unwind()
            .unwrap();

        compio_runtime::time::timeout(Duration::from_secs(2), client)
            .await
            .expect("client timed out")
            .resume_unwind()
            .unwrap();

        let _ = std::fs::remove_file(&path);
    });
}

/// IPC variant with ZMTP-like varying sizes.
#[test]
fn ipc_interleaved_zmtp_null() {
    let rt = compio_runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let path = std::env::temp_dir().join(format!(
            "compio-kq-zmtp-{}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).await.unwrap();
        let sizes: &[usize] = &[64, 32, 8];
        let path2 = path.clone();

        let server = compio_runtime::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            ipc_multi_round(&stream, b'S', b'C', sizes).await;
        });

        let client = compio_runtime::spawn(async move {
            let stream = UnixStream::connect(&path2).await.unwrap();
            ipc_multi_round(&stream, b'C', b'S', sizes).await;
        });

        compio_runtime::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("server timed out")
            .resume_unwind()
            .unwrap();

        compio_runtime::time::timeout(Duration::from_secs(2), client)
            .await
            .expect("client timed out")
            .resume_unwind()
            .unwrap();

        let _ = std::fs::remove_file(&path);
    });
}

// ---------------------------------------------------------------------------
// Read cancel-and-resubmit (driver select loop pattern)
//
// omq's driver uses select_biased! to race read readiness against outgoing
// messages and timers. When another arm fires first, the in-flight read is
// canceled (dropped). The next loop iteration resubmits a new read. On kqueue
// with EV_ONESHOT, the cancel + re-register sequence may lose the readiness
// event that arrived while the read was canceled.
// ---------------------------------------------------------------------------

/// Writer sends data after a delay; reader cancels reads via short timeouts
/// and retries, exercising the cancel-resubmit path.
#[test]
fn tcp_read_cancel_resubmit() {
    let rt = compio_runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let writer = compio_runtime::spawn(async move {
            let stream = TcpStream::connect(addr).await.unwrap();
            for round in 0u32..5 {
                compio_runtime::time::sleep(Duration::from_millis(50)).await;
                tcp_write_all(&stream, make_payload(b'W', round, 64)).await;
            }
        });

        let reader = compio_runtime::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            for round in 0u32..5 {
                let data = loop {
                    match compio_runtime::time::timeout(
                        Duration::from_millis(5),
                        tcp_read_exact(&stream, 64),
                    )
                    .await
                    {
                        Ok(data) => break data,
                        Err(_) => continue,
                    }
                };
                assert_eq!(data[0], b'W', "round {round}");
            }
        });

        compio_runtime::time::timeout(Duration::from_secs(5), writer)
            .await
            .expect("writer timed out")
            .resume_unwind()
            .unwrap();

        compio_runtime::time::timeout(Duration::from_secs(5), reader)
            .await
            .expect("reader timed out")
            .resume_unwind()
            .unwrap();
    });
}

/// Same as tcp_read_cancel_resubmit but over IPC.
#[test]
fn ipc_read_cancel_resubmit() {
    let rt = compio_runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let path = std::env::temp_dir().join(format!(
            "compio-kq-cancel-{}.sock",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).await.unwrap();
        let path2 = path.clone();

        let writer = compio_runtime::spawn(async move {
            let stream = UnixStream::connect(&path2).await.unwrap();
            for round in 0u32..5 {
                compio_runtime::time::sleep(Duration::from_millis(50)).await;
                ipc_write_all(&stream, make_payload(b'W', round, 64)).await;
            }
        });

        let reader = compio_runtime::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            for round in 0u32..5 {
                let data = loop {
                    match compio_runtime::time::timeout(
                        Duration::from_millis(5),
                        ipc_read_exact(&stream, 64),
                    )
                    .await
                    {
                        Ok(data) => break data,
                        Err(_) => continue,
                    }
                };
                assert_eq!(data[0], b'W', "round {round}");
            }
        });

        compio_runtime::time::timeout(Duration::from_secs(5), writer)
            .await
            .expect("writer timed out")
            .resume_unwind()
            .unwrap();

        compio_runtime::time::timeout(Duration::from_secs(5), reader)
            .await
            .expect("reader timed out")
            .resume_unwind()
            .unwrap();

        let _ = std::fs::remove_file(&path);
    });
}

/// Combined: interleaved handshake rounds where reads are also raced against
/// a timer. Both sides write-then-read, but the read has a short timeout.
/// Failed reads are retried. This combines the multi-round and cancel patterns.
#[test]
fn tcp_interleaved_with_cancel() {
    let rt = compio_runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = compio_runtime::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            for round in 0u32..5 {
                tcp_write_all(&stream, make_payload(b'S', round, 64)).await;
                let data = loop {
                    match compio_runtime::time::timeout(
                        Duration::from_millis(5),
                        tcp_read_exact(&stream, 64),
                    )
                    .await
                    {
                        Ok(d) => break d,
                        Err(_) => continue,
                    }
                };
                assert_eq!(data[0], b'C', "server round {round}");
            }
        });

        let client = compio_runtime::spawn(async move {
            let stream = TcpStream::connect(addr).await.unwrap();
            for round in 0u32..5 {
                tcp_write_all(&stream, make_payload(b'C', round, 64)).await;
                let data = loop {
                    match compio_runtime::time::timeout(
                        Duration::from_millis(5),
                        tcp_read_exact(&stream, 64),
                    )
                    .await
                    {
                        Ok(d) => break d,
                        Err(_) => continue,
                    }
                };
                assert_eq!(data[0], b'S', "client round {round}");
            }
        });

        compio_runtime::time::timeout(Duration::from_secs(5), server)
            .await
            .expect("server timed out")
            .resume_unwind()
            .unwrap();

        compio_runtime::time::timeout(Duration::from_secs(5), client)
            .await
            .expect("client timed out")
            .resume_unwind()
            .unwrap();
    });
}
