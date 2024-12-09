use compio_buf::{BufResult, IntoInner, IoBuf, IoVectoredBuf};

use crate::{AsyncWrite, AsyncWriteAt, IoResult};

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
    ($buf:ident, $len:expr, $needle:ident,loop $expr_expr:expr) => {
        let len = $len;
        let mut $needle = 0;

        while $needle < len {
            match $expr_expr.await.into_inner() {
                BufResult(Ok(0), buf) => {
                    return BufResult(
                        Err(::std::io::Error::new(
                            ::std::io::ErrorKind::WriteZero,
                            "failed to write whole buffer",
                        )),
                        buf,
                    );
                }
                BufResult(Ok(n), buf) => {
                    $needle += n;
                    $buf = buf;
                }
                BufResult(Err(ref e), buf) if e.kind() == ::std::io::ErrorKind::Interrupted => {
                    $buf = buf;
                }
                BufResult(Err(e), buf) => return BufResult(Err(e), buf),
            }
        }

        return BufResult(Ok(()), $buf);
    };
}

macro_rules! loop_write_vectored {
    ($buf:ident, $tracker:ident : $tracker_ty:ty, $iter:ident,loop $read_expr:expr) => {{
        use ::compio_buf::OwnedIterator;

        let mut $iter = match $buf.owned_iter() {
            Ok(buf) => buf,
            Err(buf) => return BufResult(Ok(()), buf),
        };
        let mut $tracker: $tracker_ty = 0;

        loop {
            let len = $iter.buf_len();
            if len > 0 {
                match $read_expr.await {
                    BufResult(Ok(()), ret) => {
                        $iter = ret;
                        $tracker += len as $tracker_ty;
                    }
                    BufResult(Err(e), $iter) => return BufResult(Err(e), $iter.into_inner()),
                };
            }

            match $iter.next() {
                Ok(next) => $iter = next,
                Err(buf) => return BufResult(Ok(()), buf),
            }
        }
    }};
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
            loop self.write(buf.slice(needle..))
        );
    }

    /// Write the entire contents of a buffer into this writer. Like
    /// [`AsyncWrite::write_vectored`], except that it tries to write the entire
    /// contents of the buffer into this writer.
    async fn write_vectored_all<T: IoVectoredBuf>(&mut self, buf: T) -> BufResult<(), T> {
        loop_write_vectored!(buf, _total: usize, iter, loop self.write_all(iter))
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
            loop self.write_at(buf.slice(needle..), pos + needle as u64)
        );
    }

    /// Like [`AsyncWriteAt::write_vectored_at`], expect that it tries to write
    /// the entire contents of the buffer into this writer.
    async fn write_vectored_all_at<T: IoVectoredBuf>(
        &mut self,
        buf: T,
        pos: u64,
    ) -> BufResult<(), T> {
        loop_write_vectored!(buf, total: u64, iter, loop self.write_all_at(iter, pos + total))
    }
}

impl<A: AsyncWriteAt + ?Sized> AsyncWriteAtExt for A {}
