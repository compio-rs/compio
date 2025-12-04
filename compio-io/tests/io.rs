use std::io::Cursor;

use compio_buf::{BufResult, IoBuf, IoBufMut, arrayvec::ArrayVec};
use compio_io::{
    AsyncRead, AsyncReadAt, AsyncReadAtExt, AsyncReadExt, AsyncWrite, AsyncWriteAt,
    AsyncWriteAtExt, AsyncWriteExt, split,
};
use futures_executor::block_on;

#[test]
fn io_read() {
    block_on(async {
        let mut src = &[1u8, 1, 4, 5, 1, 4, 1, 9, 1, 9, 8, 1, 0][..];
        let (len, buf) = src.read(vec![1; 6]).await.unwrap();

        assert_eq!(len, 6);
        assert_eq!(buf, [1, 1, 4, 5, 1, 4]);
        assert_eq!(src.len(), 7);
        assert_eq!(src, [1, 9, 1, 9, 8, 1, 0]);

        let (len, buf) = src.read(vec![0; 20]).await.unwrap();
        assert_eq!(len, 7);
        assert_eq!(buf.len(), 20);
        assert_eq!(&buf[..7], [1, 9, 1, 9, 8, 1, 0]);
    })
}

#[test]
fn io_write() {
    block_on(async {
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
    })
}

#[test]
fn io_write_at() {
    block_on(async {
        let mut dst = [0u8; 10];
        let (len, _) = dst.write_at(vec![1, 1, 4, 5, 1, 4], 2).await.unwrap();

        assert_eq!(len, 6);
        assert_eq!(dst, [0, 0, 1, 1, 4, 5, 1, 4, 0, 0]);

        let mut dst = [0u8; 5];
        let (len, _) = dst.write_at(vec![1, 1, 4, 5, 1, 4], 2).await.unwrap();

        assert_eq!(len, 3);
        assert_eq!(dst, [0, 0, 1, 1, 4]);

        let mut dst = [0u8; 5];
        let (len, _) = dst.write_at(vec![1, 1, 4], 6).await.unwrap();

        assert_eq!(len, 0);
        assert_eq!(dst, [0, 0, 0, 0, 0]);

        let mut dst = vec![];
        let (len, _) = dst.write_at(vec![1, 1, 4], 5).await.unwrap();

        assert_eq!(len, 3);
        assert_eq!(dst, [0, 0, 0, 0, 0, 1, 1, 4]);
    })
}

#[test]
fn io_read_at() {
    block_on(async {
        const SRC: [u8; 6] = [1, 1, 4, 5, 1, 4];

        let (len, buf) = SRC.read_at(ArrayVec::<u8, 10>::new(), 2).await.unwrap();

        assert_eq!(len, 4);
        assert_eq!(buf.as_slice(), [4, 5, 1, 4]);

        let (len, buf) = SRC.read_at(ArrayVec::<u8, 3>::new(), 2).await.unwrap();

        assert_eq!(len, 3);
        assert_eq!(buf.as_slice(), [4, 5, 1]);

        let (len, buf) = SRC.read_at(ArrayVec::<u8, 1>::new(), 7).await.unwrap();

        assert_eq!(len, 0);
        assert!(buf.as_slice().is_empty());
    })
}

#[test]
fn readv() {
    block_on(async {
        let mut src = &[1u8, 1, 4, 5, 1, 4, 1, 9, 1, 9, 8, 1, 0][..];
        let (len, buf) = src
            .read_vectored([Vec::with_capacity(6), Vec::with_capacity(4)])
            .await
            .unwrap();
        assert_eq!(len, 10);
        assert_eq!(buf[0], [1, 1, 4, 5, 1, 4]);
        assert_eq!(buf[1], [1, 9, 1, 9]);

        let mut src = &[1u8, 1, 4, 5, 1, 4, 1, 9, 1, 9, 8, 1, 0][..];
        let (len, buf) = src
            .read_vectored([vec![0; 6], Vec::with_capacity(10)])
            .await
            .unwrap();
        assert_eq!(len, 13);
        assert_eq!(buf[0], [1, 1, 4, 5, 1, 4]);
        assert_eq!(buf[1], [1, 9, 1, 9, 8, 1, 0]);

        let mut src = &[1u8, 1, 4, 5, 1, 4, 1, 9, 1, 9, 8, 1, 0][..];
        let (len, buf) = src
            .read_vectored([vec![], Vec::with_capacity(20)])
            .await
            .unwrap();
        assert_eq!(len, 13);
        assert!(buf[0].is_empty());
        assert_eq!(buf[1], [1, 1, 4, 5, 1, 4, 1, 9, 1, 9, 8, 1, 0]);
    })
}

#[test]
fn writev() {
    block_on(async {
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

        let mut dst = vec![];
        let (len, _) = dst
            .write_vectored([vec![1, 1, 4], vec![5, 1, 4]])
            .await
            .unwrap();

        assert_eq!(len, 6);
        assert_eq!(dst.len(), 6);
        assert_eq!(dst, [1, 1, 4, 5, 1, 4]);
    })
}

#[test]
fn readv_at() {
    block_on(async {
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
    })
}

#[test]
fn writev_at() {
    block_on(async {
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

        let mut dst = vec![0u8; 5];
        let (len, _) = dst
            .write_vectored_at([vec![1, 1, 4], vec![5, 1, 4]], 2)
            .await
            .unwrap();

        assert_eq!(len, 6);
        assert_eq!(dst.len(), 8);
        assert_eq!(dst, [0, 0, 1, 1, 4, 5, 1, 4]);
    })
}

struct RepeatOne(u8);

impl AsyncRead for RepeatOne {
    async fn read<B: IoBufMut>(&mut self, mut buf: B) -> BufResult<usize, B> {
        let slice = buf.as_uninit();
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
        let slice = buf.as_uninit();
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

#[test]
fn read_exact() {
    block_on(async {
        let mut src = RepeatOne(114);

        let ((), buf) = src.read_exact(vec![0; 5]).await.unwrap();
        assert_eq!(buf, [114; 5]);

        let ((), buf) = src.read_exact_at(Vec::with_capacity(5), 0).await.unwrap();
        assert_eq!(buf, [0, 114, 114, 114, 114]);

        let ((), bufs) = src
            .read_vectored_exact([vec![0; 2], vec![0; 3]])
            .await
            .unwrap();
        assert_eq!(bufs[0], [114; 2]);
        assert_eq!(bufs[1], [114; 3]);

        let ((), bufs) = src
            .read_vectored_exact_at([vec![0; 1], Vec::with_capacity(4)], 0)
            .await
            .unwrap();
        assert_eq!(bufs[0], [0]);
        assert_eq!(bufs[1], [114; 4]);
    })
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

#[test]
fn write_all() {
    block_on(async {
        let mut dst = WriteOne(vec![]);

        let ((), _) = dst.write_all([1, 1, 4, 5, 1, 4]).await.unwrap();
        assert_eq!(dst.0, [1, 1, 4, 5, 1, 4]);

        let ((), _) = dst.write_all_at([114, 114, 114], 2).await.unwrap();
        assert_eq!(dst.0, [1, 1, 114, 114, 114, 4]);

        let ((), _) = dst
            .write_vectored_all(([1u8, 9], ([8u8, 1, 0],)))
            .await
            .unwrap();
        assert_eq!(dst.0, [1, 1, 114, 114, 114, 4, 1, 9, 8, 1, 0]);

        let ((), _) = dst
            .write_vectored_all_at([[19, 19], [8, 10]], 5)
            .await
            .unwrap();
        assert_eq!(dst.0, [1, 1, 114, 114, 114, 19, 19, 8, 10, 1, 0]);
    })
}

struct ReadOne(Cursor<Vec<u8>>);

impl AsyncRead for ReadOne {
    async fn read<B: IoBufMut>(&mut self, mut buf: B) -> BufResult<usize, B> {
        let slice = buf.as_uninit();
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

#[test]
fn read_to_end() {
    block_on(async {
        let mut src = ReadOne(Cursor::new(vec![1, 1, 4, 5, 1, 4]));

        let (len, buf) = src.read_to_end(vec![]).await.unwrap();
        assert_eq!(len, 6);
        assert_eq!(buf, [1, 1, 4, 5, 1, 4]);
    })
}

#[test]
fn read_to_string() {
    block_on(async {
        let mut src = ReadOne(Cursor::new("test".to_string().into_bytes()));

        let (len, buf) = src.read_to_string(String::new()).await.unwrap();
        assert_eq!(len, 4);
        assert_eq!(buf, "test");
    })
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
            let slice = buf.as_uninit();
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

#[test]
fn read_to_end_at() {
    block_on(async {
        let src = ReadOneAt(vec![1, 1, 4, 5, 1, 4]);

        let (len, buf) = src.read_to_end_at(vec![], 2).await.unwrap();
        assert_eq!(len, 4);
        assert_eq!(buf, [4, 5, 1, 4]);
    })
}

#[test]
fn read_to_string_at() {
    block_on(async {
        let mut src = vec![1, 1];
        src.extend_from_slice("test".as_bytes());
        let mut src = ReadOneAt(src);

        let (len, buf) = src.read_to_string_at(String::new(), 2).await.unwrap();
        assert_eq!(len, 4);
        assert_eq!(buf, "test");
    })
}

#[test]
fn split_unsplit() {
    block_on(async {
        let src = Cursor::new([1, 1, 4, 5, 1, 4]);
        let (mut read, mut write) = split(src);

        let (len, buf) = read.read([0, 0, 0]).await.unwrap();
        assert_eq!(len, 3);
        assert_eq!(buf, [1, 1, 4]);

        let (len, _) = write.write([2, 2, 2]).await.unwrap();
        assert_eq!(len, 3);

        let src = read.unsplit(write);
        assert_eq!(src.into_inner(), [1, 1, 4, 2, 2, 2]);
    })
}
