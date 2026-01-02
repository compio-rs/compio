use std::io::prelude::*;

#[cfg(linux_all)]
use compio_fs::pipe::{anonymous, splice};
use compio_fs::{File, OpenOptions};
use compio_io::{AsyncReadAtExt, AsyncWriteAt, AsyncWriteAtExt};
#[cfg(linux_all)]
use compio_io::{AsyncReadExt, AsyncWriteExt};
use tempfile::NamedTempFile;

async fn setlen_run(file: &File, size: u64) {
    file.set_len(size).await.unwrap();
    // For predictability. Give the uring just enough time to ensure that it's
    // completed
    compio_runtime::time::sleep(std::time::Duration::from_millis(0)).await;

    let meta = file.metadata().await.unwrap();
    assert_eq!(size, meta.len());
}

#[compio_macros::test]
async fn iouring_setlen_non_fixed() {
    let tempfile = tempfile();
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(tempfile.path())
        .await
        .unwrap();
    setlen_run(&file, 5).await;

    setlen_run(&file, 0).await;
}

#[compio_macros::test]
async fn metadata() {
    let meta = compio_fs::metadata("Cargo.toml").await.unwrap();
    assert!(meta.is_file());
    let size = meta.len();

    let file = File::open("Cargo.toml").await.unwrap();
    let meta = file.metadata().await.unwrap();
    assert!(meta.is_file());
    assert_eq!(size, meta.len());

    let std_meta = std::fs::metadata("Cargo.toml").unwrap();
    assert_eq!(size, std_meta.len());
}

const HELLO: &[u8] = b"hello world...";

async fn read_hello(file: &File) {
    let buf = Vec::with_capacity(1024);
    let (n, buf) = file.read_to_end_at(buf, 0).await.unwrap();

    assert_eq!(n, HELLO.len());
    assert_eq!(&buf, HELLO);
}

#[compio_macros::test]
async fn basic_read() {
    let mut tempfile = tempfile();
    tempfile.write_all(HELLO).unwrap();

    let file = File::open(tempfile.path()).await.unwrap();
    read_hello(&file).await;
}

#[compio_macros::test]
async fn basic_write() {
    let tempfile = tempfile();

    let mut file = File::create(tempfile.path()).await.unwrap();

    file.write_all_at(HELLO, 0).await.0.unwrap();
    file.sync_all().await.unwrap();

    let file = std::fs::read(tempfile.path()).unwrap();
    assert_eq!(file, HELLO);
}

#[compio_macros::test]
async fn writev() {
    let tempfile = tempfile();

    let mut file = File::create(tempfile.path()).await.unwrap();

    let (write, _) = file.write_vectored_at([HELLO, HELLO], 0).await.unwrap();
    assert!(write > 0);
}

#[compio_macros::test]
async fn cancel_read() {
    let mut tempfile = tempfile();
    tempfile.write_all(HELLO).unwrap();

    let file = File::open(tempfile.path()).await.unwrap();

    // Poll the future once, then cancel it
    poll_once(async { read_hello(&file).await }).await;

    read_hello(&file).await;
}

#[cfg(unix)]
#[compio_macros::test]
async fn timeout_read() {
    use std::time::Duration;

    use compio_fs::pipe::anonymous;
    use compio_io::AsyncReadExt;
    use compio_runtime::time::timeout;

    let (mut rx, _) = anonymous().unwrap();

    // Read a file with timeout.
    let _ = timeout(Duration::from_nanos(1), async move {
        rx.read_to_string(String::new()).await
    })
    .await
    .unwrap_err();
}

#[compio_macros::test]
async fn drop_open() {
    let tempfile = tempfile();
    let _ = File::create(tempfile.path()).await;

    // Do something else
    let mut file = File::create(tempfile.path()).await.unwrap();

    file.write_all_at(HELLO, 0).await.0.unwrap();

    let file = std::fs::read(tempfile.path()).unwrap();
    assert_eq!(file, HELLO);
}

#[cfg(windows)]
#[compio_macros::test]
async fn hidden_file_truncation() {
    let tmpdir = tempfile::tempdir().unwrap();
    let path = tmpdir.path().join("hidden_file.txt");

    // Create a hidden file.
    const FILE_ATTRIBUTE_HIDDEN: u32 = 2;
    let mut file = compio_fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .attributes(FILE_ATTRIBUTE_HIDDEN)
        .open(&path)
        .await
        .unwrap();
    file.write_all_at("hidden world!", 0).await.unwrap();
    file.close().await.unwrap();

    // Create a new file by truncating the existing one.
    let file = File::create(&path).await.unwrap();
    let metadata = file.metadata().await.unwrap();
    assert_eq!(metadata.len(), 0);
}

fn tempfile() -> NamedTempFile {
    NamedTempFile::new().unwrap()
}

#[cfg(linux_all)]
#[compio_macros::test]
async fn splice_file_to_pipe() {
    let mut tempfile = tempfile();
    tempfile.write_all(HELLO).unwrap();

    let file = File::open(tempfile.path()).await.unwrap();
    let (mut rx, tx) = anonymous().unwrap();

    let n = splice(&file, &tx, HELLO.len()).offset_in(0).await.unwrap();
    assert_eq!(n, HELLO.len());

    drop(tx);
    let (_, buf) = rx
        .read_exact(Vec::with_capacity(HELLO.len()))
        .await
        .unwrap();
    assert_eq!(&buf, HELLO);
}

#[cfg(linux_all)]
#[compio_macros::test]
async fn splice_pipe_to_file() {
    let tempfile = tempfile();

    let file = compio_fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(tempfile.path())
        .await
        .unwrap();
    let (rx, mut tx) = anonymous().unwrap();

    tx.write_all(HELLO).await.unwrap();
    drop(tx);

    let n = splice(&rx, &file, HELLO.len()).offset_out(0).await.unwrap();
    assert_eq!(n, HELLO.len());

    file.sync_all().await.unwrap();
    let contents = std::fs::read(tempfile.path()).unwrap();
    assert_eq!(&contents, HELLO);
}

#[cfg(linux_all)]
#[compio_macros::test]
async fn splice_pipe_to_pipe() {
    let (rx1, mut tx1) = anonymous().unwrap();
    let (mut rx2, tx2) = anonymous().unwrap();

    tx1.write_all(HELLO).await.unwrap();
    drop(tx1);

    let n = splice(&rx1, &tx2, HELLO.len()).await.unwrap();
    assert_eq!(n, HELLO.len());

    drop(tx2);
    let (_, buf) = rx2
        .read_exact(Vec::with_capacity(HELLO.len()))
        .await
        .unwrap();
    assert_eq!(&buf, HELLO);
}

async fn poll_once(future: impl std::future::Future) {
    use std::{future::poll_fn, pin::pin, task::Poll};

    let mut future = pin!(future);

    poll_fn(|cx| {
        let _ = future.as_mut().poll(cx);
        Poll::Ready(())
    })
    .await;
}
