use compio_buf::{BufResult, IntoInner, IoBuf, IoVectoredBuf};

use crate::{AsyncWrite, AsyncWriteAt, IoResult, framed, util::Splittable};

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
    ($buf:ident, $iter:ident, $read_expr:expr) => {{
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
    async fn write_vectored_all<T: IoVectoredBuf>(&mut self, mut buf: T) -> BufResult<(), T> {
        let len = buf.total_len();
        loop_write_all!(buf, len, needle, loop self.write_vectored(buf.slice(needle)));
    }

    /// Create a [`framed::Framed`] reader/writer with the given codec and
    /// framer.
    fn framed<T, C, F>(
        self,
        codec: C,
        framer: F,
    ) -> framed::Framed<Self::ReadHalf, Self::WriteHalf, C, F, T, T>
    where
        Self: Splittable + Sized,
    {
        framed::Framed::new(codec, framer).with_duplex(self)
    }

    /// Convenience method to create a [`framed::BytesFramed`] reader/writer
    /// out of a splittable.
    #[cfg(feature = "bytes")]
    fn bytes(self) -> framed::BytesFramed<Self::ReadHalf, Self::WriteHalf>
    where
        Self: Splittable + Sized,
    {
        framed::BytesFramed::new_bytes().with_duplex(self)
    }

    /// Create a [`Splittable`] that uses `Self` as [`WriteHalf`] and `()` as
    /// [`ReadHalf`].
    ///
    /// This is useful for creating framed sink with only a writer,
    /// using the [`AsyncWriteExt::framed`] or [`AsyncWriteExt::bytes`]
    /// method, which require a [`Splittable`] to work.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use compio_io::{AsyncWriteExt, framed::BytesFramed};
    ///
    /// let mut file_bytes = file.write_only().bytes();
    /// file_bytes.send(Bytes::from("hello world")).await?;
    /// ```
    ///
    /// [`ReadHalf`]: Splittable::ReadHalf
    /// [`WriteHalf`]: Splittable::WriteHalf
    fn write_only(self) -> WriteOnly<Self>
    where
        Self: Sized,
    {
        WriteOnly(self)
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
        mut buf: T,
        pos: u64,
    ) -> BufResult<(), T> {
        let len = buf.total_len();
        loop_write_all!(buf, len, needle, loop self.write_vectored_at(buf.slice(needle), pos + needle as u64));
    }
}

impl<A: AsyncWriteAt + ?Sized> AsyncWriteAtExt for A {}

/// An adaptor which implements [`Splittable`] for any [`AsyncWrite`], with the
/// read half being `()`.
///
/// This can be used to create a framed sink with only a writer, using
/// the [`AsyncWriteExt::framed`] or [`AsyncWriteExt::bytes`] method.
pub struct WriteOnly<W>(pub W);

impl<W: AsyncWrite> AsyncWrite for WriteOnly<W> {
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        self.0.write(buf).await
    }

    async fn flush(&mut self) -> IoResult<()> {
        self.0.flush().await
    }

    async fn shutdown(&mut self) -> IoResult<()> {
        self.0.shutdown().await
    }
}

impl<W> Splittable for WriteOnly<W> {
    type ReadHalf = ();
    type WriteHalf = W;

    fn split(self) -> (Self::ReadHalf, Self::WriteHalf) {
        ((), self.0)
    }
}
