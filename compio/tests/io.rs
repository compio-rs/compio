use std::io::Cursor;

use compio_buf::arrayvec::ArrayVec;
use compio_io::{AsyncRead, AsyncReadAt, AsyncWrite, AsyncWriteAt};

#[compio_macros::test]
async fn io_read() {
    let mut src = "Hello, World";
    let (len, buf) = src.read(Vec::with_capacity(10)).await.unwrap();

    assert_eq!(len, 10);
    assert_eq!(buf, b"Hello, Wor");

    let (len, buf) = src.read(Vec::with_capacity(20)).await.unwrap();
    assert_eq!(len, 12);
    assert_eq!(buf.len(), 12);
    assert_eq!(buf, b"Hello, World");
}

#[compio_macros::test]
async fn io_write() {
    let mut dst = Cursor::new([0u8; 10]);
    let (len, _) = dst.write(vec![1, 1, 4, 5, 1, 4]).await.unwrap();

    assert_eq!(len, 6);
    assert_eq!(dst.position(), 6);
    assert_eq!(dst.into_inner(), [1, 1, 4, 5, 1, 4, 0, 0, 0, 0]);

    let mut dst = Cursor::new([0u8; 10]);
    let (len, _) = dst
        .write(vec![1, 1, 4, 5, 1, 4, 1, 9, 1, 9, 8, 1, 0])
        .await
        .unwrap();

    assert_eq!(len, 10);
    assert_eq!(dst.into_inner(), [1, 1, 4, 5, 1, 4, 1, 9, 1, 9]);
}

#[compio_macros::test]
async fn io_write_at() {
    let mut dst = [0u8; 10];
    let (len, _) = dst.write_at(vec![1, 1, 4, 5, 1, 4], 2).await.unwrap();

    assert_eq!(len, 6);
    assert_eq!(dst, [0, 0, 1, 1, 4, 5, 1, 4, 0, 0]);

    let mut dst = [0u8; 5];
    let (len, _) = dst.write_at(vec![1, 1, 4, 5, 1, 4], 2).await.unwrap();

    assert_eq!(len, 3);
    assert_eq!(dst, [0, 0, 1, 1, 4]);
}

#[compio_macros::test]
async fn io_read_at() {
    const SRC: [u8; 6] = [1, 1, 4, 5, 1, 4];

    let (len, buf) = SRC.read_at(ArrayVec::<u8, 10>::new(), 2).await.unwrap();

    assert_eq!(len, 4);
    assert_eq!(buf.as_slice(), [4, 5, 1, 4]);

    let (len, buf) = SRC.read_at(ArrayVec::<u8, 3>::new(), 2).await.unwrap();

    assert_eq!(len, 3);
    assert_eq!(buf.as_slice(), [4, 5, 1]);
}
