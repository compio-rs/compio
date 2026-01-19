#![cfg(linux_all)]

use std::env::temp_dir;

use compio_fs::{
    File,
    pipe::{anonymous, splice},
};
use compio_io::{AsyncRead, AsyncReadExt, AsyncWriteAt, AsyncWriteExt};
use compio_net::UnixStream;
use compio_runtime::Runtime;
use futures_util::future::join;
use tempfile::NamedTempFile;

const HELLO: &[u8] = b"hello world...";

async fn uds(id: u8) -> (UnixStream, UnixStream) {
    let path = temp_dir().join(format!("compio-{id}.sock"));
    let listener = compio_net::UnixListener::bind(&path).await.unwrap();
    let (a, b) = join(UnixStream::connect(&path), listener.accept()).await;
    (a.unwrap(), b.unwrap().0)
}

#[compio_macros::test]
async fn splice_uds_to_pipe() {
    let (r, mut w) = uds(1).await;
    w.write_all(HELLO).await.unwrap();

    let (mut rx, tx) = anonymous().unwrap();

    let n = splice(&r, &tx, HELLO.len()).await.unwrap();
    assert_eq!(n, HELLO.len());

    drop(tx);
    let (_, buf) = rx
        .read_exact(Vec::with_capacity(HELLO.len()))
        .await
        .unwrap();
    assert_eq!(&buf, HELLO);
}

#[compio_macros::test]
async fn splice_pipe_to_uds() {
    let (mut r, w) = uds(2).await;
    let (rx, mut tx) = anonymous().unwrap();

    tx.write_all(HELLO).await.unwrap();
    drop(tx);

    let n = splice(&rx, &w, HELLO.len()).await.unwrap();
    assert_eq!(n, HELLO.len());

    let (len, contents) = r.read(Vec::with_capacity(HELLO.len() + 10)).await.unwrap();
    assert_eq!(len, HELLO.len());
    assert_eq!(&contents, HELLO);
}

#[compio_macros::test]
async fn splice_file_to_pipe() {
    // This test only works with io_uring because splice with offset on files
    // requires io_uring support
    let driver_type = Runtime::with_current(|rt| rt.driver_type());
    if !driver_type.is_iouring() {
        return;
    }

    // Create a temporary file that will be automatically cleaned up
    let temp_file = NamedTempFile::new().unwrap();
    let temp_path = temp_file.path();

    // Create and write to the temporary file
    {
        let mut file = File::create(temp_path).await.unwrap();
        file.write_at(HELLO, 0).await.unwrap();
        file.sync_all().await.unwrap();
    }

    // Open file for reading and splice to pipe
    let file = File::open(temp_path).await.unwrap();
    let (mut rx, tx) = anonymous().unwrap();

    let n = splice(&file, &tx, HELLO.len()).offset_in(0).await.unwrap();
    assert_eq!(n, HELLO.len());

    drop(tx);
    let (_, buf) = rx
        .read_exact(Vec::with_capacity(HELLO.len()))
        .await
        .unwrap();
    assert_eq!(&buf, HELLO);

    // temp_file is automatically cleaned up when dropped
}

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
