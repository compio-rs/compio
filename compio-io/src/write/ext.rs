use compio_buf::{buf_try, BufResult, IntoInner, IoBuf, IoVectoredBuf};

use crate::{AsyncWrite, AsyncWriteAt, IoResult};

/// Shared code for write a scalar value into the underlying writer.
macro_rules! write_scalar {
    ($t:ty, $be:ident, $le:ident) => {
        ::paste::paste! {
            #[doc = concat!("Write a big endian `", stringify!($t), "` into the underlying writer.")]
            async fn [< write_ $t >](&mut self, num: $t) -> IoResult<()> {
                use ::compio_buf::{arrayvec::ArrayVec, BufResult};

                const LEN: usize = ::std::mem::size_of::<$t>();
                let BufResult(len, _) = self
                    .write_all(ArrayVec::<u8, LEN>::from(num.$be()))
                    .await;
                assert_eq!(len?, LEN, "`write_all` returned unexpected length");
                Ok(())
            }

            #[doc = concat!("Write a little endian `", stringify!($t), "` into the underlying writer.")]
            async fn [< write_ $t _le >](&mut self, num: $t) -> IoResult<()> {
                use ::compio_buf::{arrayvec::ArrayVec, BufResult};

                const LEN: usize = ::std::mem::size_of::<$t>();
                let BufResult(len, _) = self
                    .write_all(ArrayVec::<u8, LEN>::from(num.$le()))
                    .await;
                assert_eq!(len?, LEN, "`write_all` returned unexpected length");
                Ok(())
            }
        }
    };
}

/// Shared code for loop writing until all contents are written.
macro_rules! loop_write_all {
    ($buf:ident, $len:expr, $needle:ident,loop $expr_expr:expr) => {
        let len = $len;
        let mut $needle = 0;

        while $needle < len {
            let n;
            (n, $buf) = buf_try!($expr_expr.await.into_inner());
            if n == 0 {
                return BufResult(
                    Err(::std::io::Error::new(
                        ::std::io::ErrorKind::WriteZero,
                        "failed to write whole buffer",
                    )),
                    $buf,
                );
            }
            $needle += n;
        }

        return BufResult(Ok($needle), $buf);
    };
}

pub trait AsyncWriteExt: AsyncWrite {
    /// Creates a "by reference" adaptor for this instance of [`AsyncWrite`].
    ///
    /// The returned adapter also implements [`AsyncWrite`] and will simply
    /// borrow this current writer.
    fn by_ref(&mut self) -> &mut Self
    where
        Self: Sized,
    {
        self
    }

    /// Write the entire contents of a buffer into this writer.
    async fn write_all<T: IoBuf>(&mut self, mut buf: T) -> BufResult<usize, T> {
        loop_write_all!(
            buf,
            buf.buf_len(),
            needle,
            loop self.write(buf.slice(needle..))
        );
    }

    /// Write the entire contents of a buffer into this writer. Like
    /// [`AsyncWrite::write_vectored`], except that it tries to write the entire
    /// contents of the buffer into this writer.
    async fn write_all_vectored<T: IoVectoredBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        let mut iter = match buf.owned_iter() {
            Ok(iter) => iter,
            Err(buf) => return BufResult(Ok(0), buf),
        };
        let mut total = 0;

        loop {
            if iter.buf_len() == 0 {
                continue;
            }
            match self.write_all(iter).await {
                BufResult(Ok(n), ret) => {
                    iter = ret;
                    total += n;
                }
                BufResult(Err(e), ret) => return BufResult(Err(e), ret.into_inner()),
            }

            match iter.next() {
                Ok(next) => iter = next,
                Err(buf) => return BufResult(Ok(total), buf),
            }
        }
    }

    write_scalar!(u8, to_be_bytes, to_le_bytes);
    write_scalar!(u16, to_be_bytes, to_le_bytes);
    write_scalar!(u32, to_be_bytes, to_le_bytes);
    write_scalar!(u64, to_be_bytes, to_le_bytes);
    write_scalar!(u128, to_be_bytes, to_le_bytes);
    write_scalar!(i8, to_be_bytes, to_le_bytes);
    write_scalar!(i16, to_be_bytes, to_le_bytes);
    write_scalar!(i32, to_be_bytes, to_le_bytes);
    write_scalar!(i64, to_be_bytes, to_le_bytes);
    write_scalar!(i128, to_be_bytes, to_le_bytes);
    write_scalar!(f32, to_be_bytes, to_le_bytes);
    write_scalar!(f64, to_be_bytes, to_le_bytes);
}

impl<A: AsyncWrite + ?Sized> AsyncWriteExt for A {}

pub trait AsyncWriteAtExt: AsyncWriteAt {
    /// Like `write_at`, except that it tries to write the entire contents of
    /// the buffer into this writer.
    async fn write_all_at<T: IoBuf>(&mut self, mut buf: T, pos: usize) -> BufResult<usize, T> {
        loop_write_all!(
            buf,
            buf.buf_len(),
            needle,
            loop self.write_at(buf.slice(needle..), pos + needle)
        );
    }
}

impl<A: AsyncWriteAt + ?Sized> AsyncWriteAtExt for A {}
