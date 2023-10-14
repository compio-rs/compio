use std::io::prelude::*;

use compio::fs::File;
use tempfile::NamedTempFile;

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

    let file = File::open(tempfile.path()).unwrap();
    read_hello(&file).await;
}

#[compio_macros::test]
async fn basic_write() {
    let tempfile = tempfile();

    let file = File::create(tempfile.path()).unwrap();

    file.write_all_at(HELLO, 0).await.0.unwrap();
    file.sync_all().await.unwrap();

    let file = std::fs::read(tempfile.path()).unwrap();
    assert_eq!(file, HELLO);
}

#[compio_macros::test]
async fn cancel_read() {
    let mut tempfile = tempfile();
    tempfile.write_all(HELLO).unwrap();

    let file = File::open(tempfile.path()).unwrap();

    // Poll the future once, then cancel it
    poll_once(async { read_hello(&file).await }).await;

    read_hello(&file).await;
}

#[compio_macros::test]
async fn drop_open() {
    let tempfile = tempfile();
    let _ = File::create(tempfile.path());

    // Do something else
    let file = File::create(tempfile.path()).unwrap();

    file.write_all_at(HELLO, 0).await.0.unwrap();

    let file = std::fs::read(tempfile.path()).unwrap();
    assert_eq!(file, HELLO);
}

fn tempfile() -> NamedTempFile {
    NamedTempFile::new().unwrap()
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
