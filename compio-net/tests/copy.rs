use compio_io::{
    AsyncBufRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader, BufWriter,
    util::{copy, copy_bidirectional, copy_buffered, copy_with_size},
};
use compio_net::{TcpListener, TcpStream};

async fn pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (client, (server, _)) = futures_util::try_join!(
        TcpStream::connect(listener.local_addr().unwrap()),
        listener.accept()
    )
    .unwrap();
    (client, server)
}

#[cfg(target_os = "linux")]
#[test]
fn copy_falls_back_when_the_ring_cannot_fit_a_chain() {
    let mut proactor = compio_driver::Proactor::builder();
    proactor.capacity(8);
    let runtime = compio_runtime::Runtime::builder()
        .with_proactor(proactor)
        .build()
        .unwrap();
    if !runtime.driver_type().is_iouring() {
        return;
    }
    runtime.block_on(async {
        let (mut producer, mut source) = pair().await;
        let (mut destination, mut consumer) = pair().await;
        let payload: Vec<u8> = (0..128 * 1024 + 37).map(|i| (i % 251) as u8).collect();
        let (copied, (), received) = futures_util::try_join!(
            copy(&mut source, &mut destination),
            async {
                producer.write_all(payload.clone()).await.0?;
                producer.shutdown().await
            },
            async {
                let (result, buf) = consumer.read_to_end(Vec::new()).await.into_parts();
                result.map(|_| buf)
            },
        )
        .unwrap();
        assert_eq!(copied, payload.len() as u64);
        assert_eq!(received, payload);
    });
}

#[cfg(target_os = "linux")]
#[compio_macros::test]
async fn copy_with_size_bounds_the_kernel_pipe() {
    if !compio_runtime::Runtime::with_current(|rt| rt.driver_type().is_iouring()) {
        return;
    }
    let (mut producer, mut source) = pair().await;
    let (mut destination, mut consumer) = pair().await;
    // 1 MiB meets the kernel default pipe capacity on both 4K and 64K page
    // systems, so the splice path is exercised everywhere.
    let payload: Vec<u8> = (0..3 * 4096 + 37).map(|n| (n % 251) as u8).collect();
    let (copied, (), received) = futures_util::try_join!(
        copy_with_size(&mut source, &mut destination, 1024 * 1024),
        async {
            producer.write_all(payload.clone()).await.0?;
            producer.shutdown().await
        },
        async {
            let (result, buf) = consumer.read_to_end(Vec::new()).await.into_parts();
            result.map(|_| buf)
        },
    )
    .unwrap();
    assert_eq!(copied, payload.len() as u64);
    assert_eq!(received, payload);
}

#[cfg(target_os = "linux")]
#[compio_macros::test]
async fn automatic_copy_forwards_bytes_with_the_default_pipe_capacity() {
    if !compio_runtime::Runtime::with_current(|rt| rt.driver_type().is_iouring()) {
        return;
    }
    let (mut producer, mut source) = pair().await;
    let (mut destination, mut consumer) = pair().await;
    let first: Vec<u8> = (0..64 * 1024).map(|n| (n % 251) as u8).collect();
    let rest: Vec<u8> = (0..17).map(|n| (n % 239) as u8).collect();
    let (copied, (), received) = futures_util::try_join!(
        copy(&mut source, &mut destination),
        async {
            producer.write_all(first.clone()).await.0?;
            producer.write_all(rest.clone()).await.0?;
            producer.shutdown().await
        },
        async {
            let (result, first_received) =
                consumer.read_exact(vec![0; first.len()]).await.into_parts();
            result?;
            let (result, rest_received) = consumer.read_to_end(Vec::new()).await.into_parts();
            result.map(|_| (first_received, rest_received))
        },
    )
    .unwrap();
    assert_eq!(copied, (first.len() + rest.len()) as u64);
    assert_eq!(received.0, first);
    assert_eq!(received.1, rest);
}

#[compio_macros::test]
async fn bidirectional_copy_with_sizes_uses_distinct_transfer_sizes() {
    use compio_io::util::copy_bidirectional_with_sizes;

    let (mut a, relay_a) = pair().await;
    let (mut b, relay_b) = pair().await;
    let a_bytes: Vec<u8> = (0..2 * 1024 * 1024 + 37).map(|n| (n % 251) as u8).collect();
    let b_bytes = b"response completed before request".to_vec();

    let client = async {
        a.write_all(a_bytes.clone()).await.0.unwrap();
        a.shutdown().await.unwrap();
        let (_, received) = a.read_to_end(Vec::new()).await.unwrap();
        assert_eq!(received, b_bytes);
    };
    let server = async {
        let (_, received) = b.read_to_end(Vec::new()).await.unwrap();
        assert_eq!(received, a_bytes);
        b.write_all(b_bytes.clone()).await.0.unwrap();
        b.shutdown().await.unwrap();
    };
    // The large direction starts above 1 MiB while the small direction uses an
    // unusual, non-page-aligned request below it.
    let (counts, (), ()) = futures_util::join!(
        copy_bidirectional_with_sizes(&relay_a, &relay_b, 2 * 1024 * 1024, 37),
        client,
        server,
    );
    assert_eq!(counts.0.unwrap(), a_bytes.len() as u64);
    assert_eq!(counts.1.unwrap(), b_bytes.len() as u64);
}

#[compio_macros::test]
async fn copy_buffered_forces_the_requested_userspace_size() {
    let (mut producer, mut source) = pair().await;
    let (mut destination, mut consumer) = pair().await;
    let payload: Vec<u8> = (0..64 * 1024 + 17).map(|n| (n % 241) as u8).collect();
    let (copied, (), received) = futures_util::try_join!(
        copy_buffered(&mut source, &mut destination, 37),
        async {
            producer.write_all(payload.clone()).await.0?;
            producer.shutdown().await
        },
        async {
            let (result, buf) = consumer.read_to_end(Vec::new()).await.into_parts();
            result.map(|_| buf)
        },
    )
    .unwrap();
    assert_eq!(copied, payload.len() as u64);
    assert_eq!(received, payload);
}

#[compio_macros::test]
async fn bidirectional_copy_preserves_half_close_and_backpressure() {
    let (mut a, relay_a) = pair().await;
    let (mut b, relay_b) = pair().await;
    let a_bytes = b"request completed before response".to_vec();
    let b_bytes: Vec<u8> = (0..2 * 1024 * 1024 + 37).map(|n| (n % 251) as u8).collect();

    let client = async {
        a.write_all(a_bytes.clone()).await.0.unwrap();
        a.shutdown().await.unwrap();
        let (_, received) = a.read_to_end(Vec::new()).await.unwrap();
        assert_eq!(received, b_bytes);
    };
    let server = async {
        let (_, received) = b.read_to_end(Vec::new()).await.unwrap();
        assert_eq!(received, a_bytes);
        // The response starts only after EOF in the other direction. It is
        // larger than the intermediate pipe, requiring repeated drain cycles.
        b.write_all(b_bytes.clone()).await.0.unwrap();
        b.shutdown().await.unwrap();
    };
    // Borrowed halves must retain the optimization and allow independent EOFs.
    let (counts, (), ()) =
        futures_util::join!(copy_bidirectional(&relay_a, &relay_b), client, server,);
    assert_eq!(counts.0.unwrap(), a_bytes.len() as u64);
    assert_eq!(counts.1.unwrap(), b_bytes.len() as u64);
}

#[compio_macros::test]
async fn copy_preserves_buffered_prefixes_and_read_limit() {
    let (mut source, source_peer) = pair().await;
    let (destination, mut destination_peer) = pair().await;
    source.write_all("abcdefghij").await.0.unwrap();
    source.shutdown().await.unwrap();

    let mut reader = BufReader::with_capacity(32, source_peer);
    assert_eq!(reader.fill_buf().await.unwrap(), b"abcdefghij");
    reader.consume(2);
    let mut reader = reader.take(5);
    let mut writer = BufWriter::with_capacity(32, destination);
    writer.write_all("prefix:").await.0.unwrap();
    assert_eq!(copy(&mut reader, &mut writer).await.unwrap(), 5);
    let (_, received) = destination_peer.read_to_end(Vec::new()).await.unwrap();
    assert_eq!(received, b"prefix:cdefg");
    let (_, remaining) = reader.into_inner().read_to_end(Vec::new()).await.unwrap();
    assert_eq!(remaining, b"hij");
}

#[cfg(unix)]
#[compio_macros::test]
async fn copy_waits_for_nonblocking_streams() {
    use std::time::Duration;

    use compio_net::UnixStream;
    use compio_runtime::time::sleep;

    let (source, producer) = std::os::unix::net::UnixStream::pair().unwrap();
    let (destination, consumer) = std::os::unix::net::UnixStream::pair().unwrap();
    source.set_nonblocking(true).unwrap();
    destination.set_nonblocking(true).unwrap();
    let mut source = UnixStream::from_std(source).unwrap();
    let mut producer = UnixStream::from_std(producer).unwrap();
    let mut destination = UnixStream::from_std(destination).unwrap();
    let mut consumer = UnixStream::from_std(consumer).unwrap();
    let payload = vec![0xa5; 2 * 1024 * 1024 + 19];
    let (copied, (), received) = futures_util::try_join!(
        copy(&mut source, &mut destination),
        async {
            // Start copying with an empty, nonblocking source.
            sleep(Duration::from_millis(20)).await;
            producer.write_all(payload.clone()).await.0?;
            producer.shutdown().await
        },
        async {
            // Also fill the destination socket to exercise write readiness.
            sleep(Duration::from_millis(100)).await;
            let (result, buf) = consumer.read_to_end(Vec::new()).await.into_parts();
            result.map(|_| buf)
        },
    )
    .unwrap();
    assert_eq!(copied, payload.len() as u64);
    assert_eq!(received, payload);
}

#[cfg(target_os = "linux")]
#[compio_macros::test]
async fn cancelling_copy_does_not_block_closing_pipe_ends() {
    use std::time::Duration;

    use compio_runtime::time::sleep;
    use futures_util::future::{Either, select};

    let (mut source, mut peer) = pair().await;
    let (mut destination, _sink) = pair().await;
    let copying = Box::pin(copy(&mut source, &mut destination));
    let deadline = Box::pin(sleep(Duration::from_millis(20)));
    match select(copying, deadline).await {
        Either::Right(((), copying)) => drop(copying),
        Either::Left(_) => panic!("empty source unexpectedly completed"),
    }
    drop(source);
    drop(destination);
    peer.shutdown().await.unwrap();

    // The executor must still service IO after cancelling a pending splice.
    let (mut writer, mut reader) = pair().await;
    writer.write_all("still alive").await.0.unwrap();
    writer.shutdown().await.unwrap();
    assert_eq!(
        reader.read_to_end(Vec::new()).await.unwrap().1,
        b"still alive"
    );
}

#[compio_macros::test]
async fn copy_forwards_first_bytes_while_source_stays_open() {
    let (mut source, mut source_peer) = pair().await;
    let (mut destination, mut destination_peer) = pair().await;
    // Also exercise forwarding when a small socket budget limits splice
    // lengths.
    #[cfg(target_os = "linux")]
    socket2::SockRef::from(&destination)
        .set_send_buffer_size(16 * 1024)
        .unwrap();
    let (mut ctl_source, mut ctl_sink) = pair().await;
    let first: Vec<u8> = (0..128 * 1024).map(|n| (n % 251) as u8).collect();
    let rest: Vec<u8> = (0..512 * 1024 + 5).map(|n| (n % 241) as u8).collect();

    let (n, (), (forwarded, received)) = futures_util::try_join!(
        copy(&mut source, &mut destination),
        async {
            source_peer.write_all(first.clone()).await.0?;
            // Hold the connection open until the consumer has observed the
            // first bytes end to end, so forwarding must not wait for EOF.
            ctl_source.read_exact(vec![0u8; 1]).await.0?;
            source_peer.write_all(rest.clone()).await.0?;
            source_peer.shutdown().await
        },
        async {
            let (result, buf) = destination_peer
                .read_exact(vec![0u8; first.len()])
                .await
                .into_parts();
            result?;
            ctl_sink.write_all(b"!").await.0?;
            let (result, received) = destination_peer.read_to_end(Vec::new()).await.into_parts();
            result?;
            Ok((buf, received))
        },
    )
    .unwrap();
    assert_eq!(n, (first.len() + rest.len()) as u64);
    assert_eq!(
        forwarded, first,
        "first bytes must be forwarded while the source is still open"
    );
    assert_eq!(received, rest);
}

#[cfg(target_os = "linux")]
#[compio_macros::test]
async fn output_rejection_with_open_source_stops_copy() {
    use std::time::Duration;

    use compio_driver::ToSharedFd;
    use compio_runtime::time::sleep;
    use futures_util::future::{Either, pending, select};

    let test = async {
        let (mut source, mut source_peer) = pair().await;
        let (mut destination, mut destination_peer) = pair().await;
        let output = destination.to_shared_fd();
        let first: Vec<u8> = (0..128 * 1024).map(|n| (n % 251) as u8).collect();
        let mut copying = Box::pin(copy(&mut source, &mut destination));
        let mut forward = Box::pin(async {
            source_peer.write_all(first.clone()).await.0.unwrap();
            let (_, received) = destination_peer
                .read_exact(vec![0; first.len()])
                .await
                .unwrap();
            assert_eq!(received, first);
        });
        assert!(matches!(
            select(&mut forward, &mut copying).await,
            Either::Left(_)
        ));
        drop(forward);

        // Wait for the reset before supplying one final chunk. Further input
        // must not be needed to expose the failed drain to the copy future.
        socket2::SockRef::from(&destination_peer)
            .set_linger(Some(Duration::ZERO))
            .unwrap();
        drop(destination_peer);
        while socket2::SockRef::from(&output)
            .take_error()
            .unwrap()
            .is_none()
        {
            sleep(Duration::from_millis(1)).await;
        }
        let feed = Box::pin(async {
            source_peer.write_all(vec![7; 64 * 1024]).await.0.unwrap();
            pending::<()>().await;
        });
        let result = match select(copying, feed).await {
            Either::Left((result, feed)) => {
                drop(feed);
                result
            }
            Either::Right(_) => unreachable!(),
        };
        let error = result.unwrap_err();
        assert!(
            matches!(
                error.kind(),
                std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
            ),
            "unexpected error: {error:?}"
        );
        drop(source_peer);
        drop(source);
        drop(destination);
        drop(output);
        let (mut writer, mut reader) = pair().await;
        writer.write_all("still alive").await.0.unwrap();
        writer.shutdown().await.unwrap();
        assert_eq!(
            reader.read_to_end(Vec::new()).await.unwrap().1,
            b"still alive"
        );
    };
    assert!(matches!(
        select(Box::pin(test), Box::pin(sleep(Duration::from_secs(10)))).await,
        Either::Left(_)
    ));
}

#[cfg(target_os = "linux")]
#[compio_macros::test]
async fn cancelling_copy_after_partial_progress_keeps_runtime_live() {
    use std::time::Duration;

    use compio_runtime::time::sleep;
    use futures_util::future::{Either, select};

    let (mut source, mut source_peer) = pair().await;
    let (mut destination, mut destination_peer) = pair().await;
    let payload: Vec<u8> = (0..32 * 1024).map(|n| (n % 251) as u8).collect();

    // Buffer input without EOF so the copy stays mid-stream.
    source_peer.write_all(payload.clone()).await.0.unwrap();
    let copying = Box::pin(copy(&mut source, &mut destination));
    let progress = Box::pin(async {
        let (result, buf) = destination_peer
            .read_exact(vec![0u8; 8 * 1024])
            .await
            .into_parts();
        result.unwrap();
        buf
    });
    let deadline = Box::pin(sleep(Duration::from_secs(10)));
    let (forwarded, copying) = match select(copying, select(progress, deadline)).await {
        Either::Left(_) => panic!("copy finished before source EOF"),
        Either::Right((Either::Left((buf, _)), copying)) => (buf, copying),
        Either::Right((Either::Right(_), _)) => panic!("copy made no progress"),
    };
    // Whatever was delivered before cancellation must be an exact prefix.
    assert_eq!(&forwarded[..], &payload[..8 * 1024]);

    // Cancel with a partially completed batch; the source never reached EOF.
    drop(copying);
    drop(source);
    drop(destination);

    // The executor must still service IO after cancelling pending splices.
    let (mut writer, mut reader) = pair().await;
    let roundtrip = Box::pin(async {
        writer.write_all("still alive").await.0.unwrap();
        writer.shutdown().await.unwrap();
        assert_eq!(
            reader.read_to_end(Vec::new()).await.unwrap().1,
            b"still alive"
        );
    });
    let deadline = Box::pin(sleep(Duration::from_secs(10)));
    assert!(
        matches!(select(roundtrip, deadline).await, Either::Left(_)),
        "runtime stalled after cancelling a partially completed copy"
    );
}
