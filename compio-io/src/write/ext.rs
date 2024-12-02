use compio_buf::{BufResult, IntoInner, IoBuf, IoVectoredBuf};

use crate::{AsyncWrite, AsyncWriteAt, IoResult, vectored::VectoredWrap};

/// Shared code for write a scalar value into the underlying writer.
macro_rules! write_scalar {
    ($t:ty, $be:ident, $le:ident) => {
        ::paste::paste! {
            #[doc = concat!("Write a big endian `", stringify!($t), "` into the underlying writer.")]
            async fn [< write_ $t >](&mut self, num: $t) -> IoResult<()> {
                use ::compio_buf::{arrayvec::ArrayVec, BufResult};

                const LEN: usize = ::std::mem::size_of::<$t>();
                let BufResult(res, _) = self
                    .write_all(ArrayVec::<u8, LEN>::from(num.$be()))
                    .await;
                res
            }

            #[doc = concat!("Write a little endian `", stringify!($t), "` into the underlying writer.")]
            async fn [< write_ $t _le >](&mut self, num: $t) -> IoResult<()> {
                use ::compio_buf::{arrayvec::ArrayVec, BufResult};

                const LEN: usize = ::std::mem::size_of::<$t>();
                let BufResult(res, _) = self
                    .write_all(ArrayVec::<u8, LEN>::from(num.$le()))
                    .await;
                res
            }
        }
    };
}

/// Shared code for loop writing until all contents are written.
macro_rules! loop_write_all {
    ($buf:ident, $len:expr, $tracker:ident, $write_expr:expr, $buf_expr:expr) => {
        let mut $tracker = 0usize;
        let len = $len;
        while $tracker < len {
            let BufResult(res, buf) = $write_expr;
            $buf = buf;
            match res {
                Ok(0) => {
                    return BufResult(
                        Err(::std::io::Error::new(
                            ::std::io::ErrorKind::WriteZero,
                            "failed to write whole buffer",
                        )),
                        $buf_expr,
                    );
                }
                Ok(n) => {
                    $tracker += n as usize;
                }
                Err(ref e) if e.kind() == ::std::io::ErrorKind::Interrupted => {}
                Err(e) => return BufResult(Err(e), $buf_expr),
            }
        }

        return BufResult(Ok(()), $buf_expr);
    };
}

macro_rules! loop_write_vectored {
    ($buf:ident, $iter:ident, $read_expr:expr) => {{
        use ::compio_buf::OwnedIterator;

        let mut $iter = match $buf.owned_iter() {
            Ok(buf) => buf,
            Err(buf) => return BufResult(Ok(0), buf),
        };

        loop {
            if $iter.buf_len() > 0 {
                return $read_expr.await.into_inner();
            }

            match $iter.next() {
                Ok(next) => $iter = next,
                Err(buf) => return BufResult(Ok(0), buf),
            }
        }
    }};
}

/// Implemented as an extension trait, adding utility methods to all
/// [`AsyncWrite`] types. Callers will tend to import this trait instead of
/// [`AsyncWrite`].
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
    async fn write_all<T: IoBuf>(&mut self, mut buf: T) -> BufResult<(), T> {
        loop_write_all!(
            buf,
            buf.buf_len(),
            needle,
            self.write(buf.slice(needle..)).await.into_inner(),
            buf
        );
    }

    /// Write the entire contents of a buffer into this writer. Like
    /// [`AsyncWrite::write_vectored`], except that it tries to write the entire
    /// contents of the buffer into this writer.
    async fn write_vectored_all<T: IoVectoredBuf>(&mut self, buf: T) -> BufResult<(), T> {
        let mut buf = VectoredWrap::new(buf);
        loop_write_all!(
            buf,
            buf.len(),
            needle,
            self.write_vectored(buf).await,
            buf.into_inner()
        );
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

/// Implemented as an extension trait, adding utility methods to all
/// [`AsyncWriteAt`] types. Callers will tend to import this trait instead of
/// [`AsyncWriteAt`].
pub trait AsyncWriteAtExt: AsyncWriteAt {
    /// Like [`AsyncWriteAt::write_at`], except that it tries to write the
    /// entire contents of the buffer into this writer.
    async fn write_all_at<T: IoBuf>(&mut self, mut buf: T, pos: u64) -> BufResult<(), T> {
        loop_write_all!(
            buf,
            buf.buf_len(),
            needle,
            self.write_at(buf.slice(needle..), pos + needle as u64)
                .await
                .into_inner(),
            buf
        );
    }

    /// Like [`AsyncWriteAt::write_vectored_at`], expect that it tries to write
    /// the entire contents of the buffer into this writer.
    async fn write_vectored_all_at<T: IoVectoredBuf>(
        &mut self,
        buf: T,
        pos: u64,
    ) -> BufResult<(), T> {
        let mut buf = VectoredWrap::new(buf);
        loop_write_all!(
            buf,
            buf.len(),
            needle,
            self.write_vectored_at(buf, pos + needle as u64).await,
            buf.into_inner()
        );
    }
}

impl<A: AsyncWriteAt + ?Sized> AsyncWriteAtExt for A {}
