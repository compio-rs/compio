use std::io::Cursor;

use compio::{
    buf::{arrayvec::ArrayVec, IoBuf, IoBufMut},
    io::{
        AsyncRead, AsyncReadAt, AsyncReadAtExt, AsyncReadExt, AsyncWrite, AsyncWriteAt,
        AsyncWriteAtExt, AsyncWriteExt,
    },
    BufResult,
};

#[compio_macros::test]
async fn io_read() {
    let mut src = "Hello, World";
    let (len, buf) = src.read(vec![1; 10]).await.unwrap();

    assert_eq!(len, 10);
    assert_eq!(buf, b"Hello, Wor");

    let (len, buf) = src.read(vec![0; 20]).await.unwrap();
    assert_eq!(len, 12);
    assert_eq!(buf.len(), 20);
    assert_eq!(&buf[..12], b"Hello, World");
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

#[compio_macros::test]
async fn readv() {
    let mut src = "Hello, world";
    let (len, buf) = src
        .read_vectored([Vec::with_capacity(5), Vec::with_capacity(5)])
        .await
        .unwrap();
    assert_eq!(len, 10);
    assert_eq!(buf[0], b"Hello");
    assert_eq!(buf[1], b", wor");

    let (len, buf) = src
        .read_vectored([vec![0; 5], Vec::with_capacity(10)])
        .await
        .unwrap();
    assert_eq!(len, 12);
    assert_eq!(buf[0], b"Hello");
    assert_eq!(buf[1], b", world");

    let (len, buf) = src
        .read_vectored([vec![], Vec::with_capacity(20)])
        .await
        .unwrap();
    assert_eq!(len, 12);
    assert!(buf[0].is_empty());
    assert_eq!(buf[1], b"Hello, world");
}

#[compio_macros::test]
async fn writev() {
    let mut dst = Cursor::new([0u8; 10]);
    let (len, _) = dst
        .write_vectored([vec![1, 1, 4], vec![5, 1, 4]])
        .await
        .unwrap();

    assert_eq!(len, 6);
    assert_eq!(dst.position(), 6);
    assert_eq!(dst.into_inner(), [1, 1, 4, 5, 1, 4, 0, 0, 0, 0]);

    let mut dst = Cursor::new([0u8; 10]);
    let (len, _) = dst
        .write_vectored([vec![1, 1, 4, 5, 1, 4], vec![1, 9, 1, 9, 8, 1, 0]])
        .await
        .unwrap();

    assert_eq!(len, 10);
    assert_eq!(dst.into_inner(), [1, 1, 4, 5, 1, 4, 1, 9, 1, 9]);
}

#[compio_macros::test]
async fn readv_at() {
    const SRC: [u8; 6] = [1, 1, 4, 5, 1, 4];

    let (len, buf) = SRC
        .read_vectored_at([ArrayVec::<u8, 5>::new(), ArrayVec::<u8, 5>::new()], 2)
        .await
        .unwrap();

    assert_eq!(len, 4);
    assert_eq!(buf[0].as_slice(), [4, 5, 1, 4]);
    assert!(buf[1].is_empty());

    let (len, buf) = SRC
        .read_vectored_at([vec![0; 3], Vec::with_capacity(1)], 2)
        .await
        .unwrap();

    assert_eq!(len, 4);
    assert_eq!(buf[0].as_slice(), [4, 5, 1]);
    assert_eq!(buf[1].as_slice(), [4]);
}

#[compio_macros::test]
async fn writev_at() {
    let mut dst = [0u8; 10];
    let (len, _) = dst
        .write_vectored_at([vec![1, 1, 4], vec![5, 1, 4]], 2)
        .await
        .unwrap();

    assert_eq!(len, 6);
    assert_eq!(dst, [0, 0, 1, 1, 4, 5, 1, 4, 0, 0]);

    let mut dst = [0u8; 5];
    let (len, _) = dst
        .write_vectored_at([vec![1, 1, 4], vec![5, 1, 4]], 2)
        .await
        .unwrap();

    assert_eq!(len, 3);
    assert_eq!(dst, [0, 0, 1, 1, 4]);
}

struct RepeatOne(u8);

impl AsyncRead for RepeatOne {
    async fn read<B: IoBufMut>(&mut self, mut buf: B) -> BufResult<usize, B> {
        let slice = buf.as_mut_slice();
        if !slice.is_empty() {
            slice[0].write(self.0);
            unsafe { buf.set_buf_init(1) };
            BufResult(Ok(1), buf)
        } else {
            BufResult(Ok(0), buf)
        }
    }
}

impl AsyncReadAt for RepeatOne {
    async fn read_at<T: IoBufMut>(&self, mut buf: T, pos: u64) -> BufResult<usize, T> {
        let slice = buf.as_mut_slice();
        if !slice.is_empty() {
            if pos == 0 {
                slice[0].write(0);
            } else {
                slice[0].write(self.0);
            }
            unsafe { buf.set_buf_init(1) };
            BufResult(Ok(1), buf)
        } else {
            BufResult(Ok(0), buf)
        }
    }
}

#[compio_macros::test]
async fn read_exact() {
    let mut src = RepeatOne(114);

    let (len, buf) = src.read_exact(Vec::with_capacity(5)).await.unwrap();
    assert_eq!(len, 5);
    assert_eq!(buf, [114; 5]);

    let (len, buf) = src.read_exact_at(Vec::with_capacity(5), 0).await.unwrap();
    assert_eq!(len, 5);
    assert_eq!(buf, [0, 114, 114, 114, 114]);
}

struct WriteOne(Vec<u8>);

impl AsyncWrite for WriteOne {
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        let slice = buf.as_slice();
        if !slice.is_empty() {
            self.0.push(slice[0]);
            BufResult(Ok(1), buf)
        } else {
            BufResult(Ok(0), buf)
        }
    }

    async fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }

    async fn shutdown(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl AsyncWriteAt for WriteOne {
    async fn write_at<T: IoBuf>(&mut self, buf: T, pos: u64) -> BufResult<usize, T> {
        let pos = pos as usize;
        if pos > self.0.len() {
            BufResult(
                Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "invalid position",
                )),
                buf,
            )
        } else {
            let slice = buf.as_slice();
            if !slice.is_empty() {
                if pos == self.0.len() {
                    self.0.push(slice[0]);
                } else {
                    self.0[pos] = slice[0];
                }
                BufResult(Ok(1), buf)
            } else {
                BufResult(Ok(0), buf)
            }
        }
    }
}

#[compio_macros::test]
async fn write_all() {
    let mut dst = WriteOne(vec![]);

    let (len, _) = dst.write_all([1, 1, 4, 5, 1, 4]).await.unwrap();
    assert_eq!(len, 6);
    assert_eq!(dst.0, [1, 1, 4, 5, 1, 4]);

    let (len, _) = dst.write_all_at([114, 114, 114], 2).await.unwrap();
    assert_eq!(len, 3);
    assert_eq!(dst.0, [1, 1, 114, 114, 114, 4]);
}

struct ReadOne(Cursor<Vec<u8>>);

impl AsyncRead for ReadOne {
    async fn read<B: IoBufMut>(&mut self, mut buf: B) -> BufResult<usize, B> {
        let slice = buf.as_mut_slice();
        if !slice.is_empty() {
            let ob = [0];
            match self.0.read(ob).await {
                BufResult(Ok(res), ob) => {
                    if res == 0 {
                        BufResult(Ok(0), buf)
                    } else {
                        slice[0].write(ob[0]);
                        unsafe { buf.set_buf_init(1) };
                        BufResult(Ok(1), buf)
                    }
                }
                BufResult(Err(e), _) => BufResult(Err(e), buf),
            }
        } else {
            BufResult(Ok(0), buf)
        }
    }
}

#[compio_macros::test]
async fn read_to_end() {
    let mut src = ReadOne(Cursor::new(vec![1, 1, 4, 5, 1, 4]));

    let (len, buf) = src.read_to_end(vec![]).await.unwrap();
    assert_eq!(len, 6);
    assert_eq!(buf, [1, 1, 4, 5, 1, 4]);
}

struct ReadOneAt(Vec<u8>);

impl AsyncReadAt for ReadOneAt {
    async fn read_at<T: IoBufMut>(&self, mut buf: T, pos: u64) -> BufResult<usize, T> {
        let pos = pos as usize;
        if pos > self.0.len() {
            BufResult(
                Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "invalid position",
                )),
                buf,
            )
        } else {
            let slice = buf.as_mut_slice();
            if !slice.is_empty() && pos < self.0.len() {
                slice[0].write(self.0[pos]);
                unsafe { buf.set_buf_init(1) };
                BufResult(Ok(1), buf)
            } else {
                BufResult(Ok(0), buf)
            }
        }
    }
}

#[compio_macros::test]
async fn read_to_end_at() {
    let src = ReadOneAt(vec![1, 1, 4, 5, 1, 4]);

    let (len, buf) = src.read_to_end_at(vec![], 2).await.unwrap();
    assert_eq!(len, 4);
    assert_eq!(buf, [4, 5, 1, 4]);
}
