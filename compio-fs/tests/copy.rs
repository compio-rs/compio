#![cfg(target_os = "linux")]

use std::io::{Seek, SeekFrom, Write};

use compio_buf::{BufResult, IoBuf};
use compio_driver::ToSharedFd;
use compio_io::{
    AsyncReadExt, AsyncWrite, AsyncWriteExt,
    util::{copy, copy_with_size},
};
use compio_runtime::{Runtime, fd::AsyncFd};

// An append-only writer can safely expose its descriptor: both write paths
// append, regardless of offsets. Linux nevertheless rejects splice to O_APPEND.
struct AppendWriter(AsyncFd<std::fs::File>);

impl AsyncWrite for AppendWriter {
    async fn write<B: IoBuf>(&mut self, buf: B) -> BufResult<usize, B> {
        self.0.write(buf).await
    }

    async fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush().await
    }

    async fn shutdown(&mut self) -> std::io::Result<()> {
        self.0.shutdown().await
    }

    fn copy_fd(&self) -> Option<impl std::os::fd::AsFd + 'static> {
        Some(self.0.to_shared_fd())
    }
}

#[compio_macros::test]
async fn copy_to_append_file_preserves_pipe_bytes() {
    if !Runtime::with_current(|rt| rt.driver_type().is_iouring()) {
        return;
    }
    let payload: Vec<u8> = (0..256 * 1024 + 13).map(|n| (n % 251) as u8).collect();
    let (mut reader, mut producer) = compio_fs::pipe::anonymous().await.unwrap();
    let destination = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(destination.path(), b"prefix:").unwrap();
    let append = std::fs::OpenOptions::new()
        .append(true)
        .open(destination.path())
        .unwrap();
    let mut writer = AppendWriter(AsyncFd::new(append).unwrap());
    let ((), n) = futures_util::join!(
        async {
            producer.write_all(payload.clone()).await.0.unwrap();
            drop(producer);
        },
        copy(&mut reader, &mut writer),
    );
    assert_eq!(n.unwrap(), payload.len() as u64);
    let mut expected = b"prefix:".to_vec();
    expected.extend_from_slice(&payload);
    assert_eq!(std::fs::read(destination.path()).unwrap(), expected);
}

#[compio_macros::test]
async fn seekable_writer_preserves_buffered_copy_offsets() {
    if !Runtime::with_current(|rt| rt.driver_type().is_iouring()) {
        return;
    }
    let mut actual = tempfile::NamedTempFile::new().unwrap();
    let mut expected = tempfile::NamedTempFile::new().unwrap();
    for file in [&mut actual, &mut expected] {
        file.write_all(b"0123456789").unwrap();
        file.seek(SeekFrom::Start(4)).unwrap();
    }
    let mut actual_writer = AsyncFd::new(actual.as_file().try_clone().unwrap()).unwrap();
    let mut expected_writer = AsyncFd::new(expected.as_file().try_clone().unwrap()).unwrap();
    let (mut reader, mut producer) = compio_fs::pipe::anonymous().await.unwrap();
    producer.write_all("xyz").await.0.unwrap();
    drop(producer);
    assert_eq!(copy(&mut reader, &mut actual_writer).await.unwrap(), 3);
    copy_with_size(&mut b"xyz".as_slice(), &mut expected_writer, 8192)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(actual.path()).unwrap(),
        std::fs::read(expected.path()).unwrap()
    );
    assert_eq!(
        actual.stream_position().unwrap(),
        expected.stream_position().unwrap()
    );
}

#[compio_macros::test]
async fn pipe_copy_handles_multiple_pipe_capacities() {
    let (mut source, mut producer) = compio_fs::pipe::anonymous().await.unwrap();
    let (mut consumer, mut destination) = compio_fs::pipe::anonymous().await.unwrap();
    let payload: Vec<u8> = (0..512 * 1024 + 1).map(|n| (n % 239) as u8).collect();
    let ((), n, received) = futures_util::join!(
        async {
            producer.write_all(payload.clone()).await.0.unwrap();
            drop(producer);
        },
        async {
            let n = copy(&mut source, &mut destination).await.unwrap();
            drop(destination);
            n
        },
        async { consumer.read_to_end(Vec::new()).await.unwrap().1 },
    );
    assert_eq!(n, payload.len() as u64);
    assert_eq!(received, payload);
}

#[compio_macros::test]
async fn late_append_fallback_preserves_bytes_while_producer_interacts() {
    if !Runtime::with_current(|rt| rt.driver_type().is_iouring()) {
        return;
    }
    let payload: Vec<u8> = (0..256 * 1024 + 13).map(|n| (n % 251) as u8).collect();
    let (mut reader, mut producer) = compio_fs::pipe::anonymous().await.unwrap();
    let destination = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(destination.path(), b"prefix:").unwrap();
    let append = std::fs::OpenOptions::new()
        .append(true)
        .open(destination.path())
        .unwrap();
    let mut writer = AppendWriter(AsyncFd::new(append).unwrap());
    let file = destination.path().to_owned();
    let ((), n) = futures_util::join!(
        async {
            use std::time::{Duration, Instant};

            use compio_runtime::time::sleep;

            let deadline = Instant::now() + Duration::from_secs(10);

            // Deliver the first chunk while the source stays open. The drain
            // through the unsupported append writer must preserve those bytes
            // before EOF is signalled, so a late fallback cannot strand
            // already consumed pipe data.
            producer
                .write_all(payload[..32 * 1024].to_vec())
                .await
                .0
                .unwrap();
            while std::fs::metadata(&file).unwrap().len() < (b"prefix:".len() + 32 * 1024) as u64 {
                assert!(
                    Instant::now() < deadline,
                    "copy never forwarded the confirmed chunk"
                );
                sleep(Duration::from_millis(1)).await;
            }
            assert_eq!(
                &std::fs::read(&file).unwrap()[b"prefix:".len()..],
                &payload[..32 * 1024],
                "confirmed chunk must reach the file while the source is open"
            );
            producer
                .write_all(payload[32 * 1024..].to_vec())
                .await
                .0
                .unwrap();
            drop(producer);
        },
        copy(&mut reader, &mut writer),
    );
    assert_eq!(n.unwrap(), payload.len() as u64);
    let mut expected = b"prefix:".to_vec();
    expected.extend_from_slice(&payload);
    assert_eq!(std::fs::read(&file).unwrap(), expected);
}
