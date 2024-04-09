use std::io::Cursor;

use compio_io::compat::AsyncStream;
use futures_util::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn async_compat_read() {
    let src = &[1u8, 1, 4, 5, 1, 4, 1, 9, 1, 9, 8, 1, 0][..];
    let mut stream = AsyncStream::new(src);

    let mut buf = [0; 6];
    let len = stream.read(&mut buf).await.unwrap();

    assert_eq!(len, 6);
    assert_eq!(buf, [1, 1, 4, 5, 1, 4]);

    let mut buf = [0; 20];
    let len = stream.read(&mut buf).await.unwrap();
    assert_eq!(len, 7);
    assert_eq!(&buf[..7], [1, 9, 1, 9, 8, 1, 0]);
}

#[tokio::test]
async fn async_compat_write() {
    let dst = Cursor::new([0u8; 10]);
    let mut stream = AsyncStream::new(dst);

    let len = stream.write(&[1, 1, 4, 5, 1, 4]).await.unwrap();
    stream.flush().await.unwrap();

    assert_eq!(len, 6);
    assert_eq!(stream.get_ref().position(), 6);
    assert_eq!(stream.get_ref().get_ref(), &[1, 1, 4, 5, 1, 4, 0, 0, 0, 0]);

    let dst = Cursor::new([0u8; 10]);
    let mut stream = AsyncStream::with_capacity(10, dst);
    let len = stream
        .write(&[1, 1, 4, 5, 1, 4, 1, 9, 1, 9, 8, 1, 0])
        .await
        .unwrap();
    assert_eq!(len, 10);

    stream.flush().await.unwrap();
    assert_eq!(stream.get_ref().get_ref(), &[1, 1, 4, 5, 1, 4, 1, 9, 1, 9]);
}

#[tokio::test]
async fn async_compat_flush_fail() {
    let dst = Cursor::new([0u8; 10]);
    let mut stream = AsyncStream::new(dst);
    let len = stream
        .write(&[1, 1, 4, 5, 1, 4, 1, 9, 1, 9, 8, 1, 0])
        .await
        .unwrap();
    assert_eq!(len, 13);
    let err = stream.flush().await.unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::UnexpectedEof);
}
