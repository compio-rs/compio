use std::io::Cursor;

use compio_io::{AsyncRead, AsyncWrite};

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
