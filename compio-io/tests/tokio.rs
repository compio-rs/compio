use std::io::Cursor;

use compio_io::compat::{AsyncReadStream, AsyncWriteStream};
use futures_executor::block_on;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};

#[test]
fn tokio_compat_read() {
    block_on(async {
        let src = &[1u8, 1, 4, 5, 1, 4, 1, 9, 1, 9, 8, 1, 0][..];
        let mut stream = std::pin::pin!(AsyncReadStream::new(src));

        let mut buf = [0; 6];
        stream.read_exact(&mut buf).await.unwrap();
        assert_eq!(buf, [1, 1, 4, 5, 1, 4]);

        let mut rest = Vec::new();
        stream.read_to_end(&mut rest).await.unwrap();
        assert_eq!(rest, [1, 9, 1, 9, 8, 1, 0]);
    })
}

#[test]
fn tokio_compat_read_into_uninit() {
    block_on(async {
        let src = &b"hello"[..];
        let mut stream = std::pin::pin!(AsyncReadStream::new(src));

        // `read_buf` hands the stream uninitialized spare capacity.
        let mut buf = Vec::with_capacity(16);
        let len = stream.read_buf(&mut buf).await.unwrap();
        assert_eq!(len, 5);
        assert_eq!(buf, b"hello");
    })
}

#[test]
fn tokio_compat_bufread() {
    block_on(async {
        let src = &b"first\nsecond\n"[..];
        let mut stream = std::pin::pin!(AsyncReadStream::new(src));

        let mut line = String::new();
        stream.read_line(&mut line).await.unwrap();
        assert_eq!(line, "first\n");
        line.clear();
        stream.read_line(&mut line).await.unwrap();
        assert_eq!(line, "second\n");
    })
}

#[test]
fn tokio_compat_write() {
    block_on(async {
        let dst = Cursor::new([0u8; 10]);
        let mut stream = std::pin::pin!(AsyncWriteStream::new(dst));

        stream.write_all(&[1, 1, 4, 5, 1, 4]).await.unwrap();
        stream.flush().await.unwrap();
        assert_eq!(stream.get_ref().get_ref(), &[1, 1, 4, 5, 1, 4, 0, 0, 0, 0]);

        stream.shutdown().await.unwrap();
    })
}
