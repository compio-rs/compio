use std::io::Write;

use compio_fs::File;
#[cfg(unix)]
use compio_fs::pipe;
use compio_io::AsyncReadManagedAt;
#[cfg(unix)]
use compio_io::{AsyncReadManaged, AsyncReadMulti, AsyncWriteExt};
#[cfg(unix)]
use futures_util::TryStreamExt;
use tempfile::NamedTempFile;

const HELLO: &[u8] = b"hello world...";

fn tempfile() -> NamedTempFile {
    NamedTempFile::new().unwrap()
}

#[compio_macros::test]
async fn test_read_file() {
    let mut tempfile = tempfile();
    tempfile.write_all(HELLO).unwrap();

    let file = File::open(tempfile.path()).await.unwrap();
    let buf = file.read_managed_at(0, 0).await.unwrap().unwrap();

    assert_eq!(buf.len(), HELLO.len());
    assert_eq!(buf.as_ref(), HELLO);
}

#[cfg(unix)]
#[compio_macros::test]
async fn test_read_pipe() {
    let (mut rx, mut tx) = pipe::anonymous().await.unwrap();
    tx.write_all(HELLO).await.unwrap();

    let buf = rx.read_managed(0).await.unwrap().unwrap();

    assert_eq!(buf.len(), HELLO.len());
    assert_eq!(buf.as_ref(), HELLO);
}

#[cfg(unix)]
#[compio_macros::test]
async fn test_read_pipe_multi() {
    let (mut rx, mut tx) = pipe::anonymous().await.unwrap();
    tx.write_all(HELLO).await.unwrap();

    let buffer = rx.read_multi(0).try_collect::<Vec<_>>().await.unwrap();
    assert_eq!(buffer.len(), 1);
    assert_eq!(&*buffer[0], HELLO);
}
