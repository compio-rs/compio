use std::io::{Cursor, Read};

use compio_io::compat::{AsyncReadStream, AsyncWriteStream, SyncStream};
use futures_executor::block_on;
use futures_util::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};

#[test]
fn async_compat_read() {
    block_on(async {
        let src = &[1u8, 1, 4, 5, 1, 4, 1, 9, 1, 9, 8, 1, 0][..];
        let stream = AsyncReadStream::new(src);
        let mut stream = std::pin::pin!(stream);

        let mut buf = [0; 6];
        let len = stream.read(&mut buf).await.unwrap();

        assert_eq!(len, 6);
        assert_eq!(buf, [1, 1, 4, 5, 1, 4]);

        let mut buf = [0; 20];
        let len = stream.read(&mut buf).await.unwrap();
        assert_eq!(len, 7);
        assert_eq!(&buf[..7], [1, 9, 1, 9, 8, 1, 0]);
    })
}

#[test]
fn async_compat_bufread() {
    block_on(async {
        let src = &[1u8, 1, 4, 5, 1, 4, 1, 9, 1, 9, 8, 1, 0][..];
        let stream = AsyncReadStream::new(src);
        let mut stream = std::pin::pin!(stream);

        let slice = stream.fill_buf().await.unwrap();
        assert_eq!(slice, [1, 1, 4, 5, 1, 4, 1, 9, 1, 9, 8, 1, 0]);
        stream.consume_unpin(6);

        let mut buf = [0; 7];
        let len = stream.read(&mut buf).await.unwrap();

        assert_eq!(len, 7);
        assert_eq!(buf, [1, 9, 1, 9, 8, 1, 0]);
    })
}

#[test]
fn async_compat_write() {
    block_on(async {
        let dst = Cursor::new([0u8; 10]);
        let stream = AsyncWriteStream::new(dst);
        let mut stream = std::pin::pin!(stream);

        let len = stream.write(&[1, 1, 4, 5, 1, 4]).await.unwrap();
        stream.flush().await.unwrap();

        assert_eq!(len, 6);
        assert_eq!(stream.get_ref().position(), 6);
        assert_eq!(stream.get_ref().get_ref(), &[1, 1, 4, 5, 1, 4, 0, 0, 0, 0]);

        let dst = Cursor::new([0u8; 10]);
        let stream = AsyncWriteStream::with_capacity(10, dst);
        let mut stream = std::pin::pin!(stream);

        let len = stream
            .write(&[1, 1, 4, 5, 1, 4, 1, 9, 1, 9, 8, 1, 0])
            .await
            .unwrap();
        assert_eq!(len, 13);

        let err = stream.flush().await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::WriteZero);
    })
}

#[test]
fn async_compat_flush_fail() {
    block_on(async {
        let dst = Cursor::new([0u8; 10]);
        let stream = AsyncWriteStream::new(dst);
        let mut stream = std::pin::pin!(stream);
        let len = stream
            .write(&[1, 1, 4, 5, 1, 4, 1, 9, 1, 9, 8, 1, 0])
            .await
            .unwrap();
        assert_eq!(len, 13);
        let err = stream.flush().await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::WriteZero);
    })
}

#[test]
fn sync_stream_into_parts_keeps_unread_buffer() {
    let mut stream = SyncStream::new(Cursor::new(b"hello".to_vec()));
    let mut buf = [0; 2];

    Read::read(&mut stream, &mut buf).unwrap_err();
    futures_executor::block_on(stream.fill_read_buf()).unwrap();
    assert_eq!(Read::read(&mut stream, &mut buf).unwrap(), 2);
    assert_eq!(&buf, b"he");

    let (_, remaining) = stream.into_parts();
    assert_eq!(remaining, b"llo");
}
