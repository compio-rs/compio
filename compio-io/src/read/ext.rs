use compio_buf::{buf_try, BufResult, IntoInner, IoBufMut, IoVectoredBufMut};

use crate::{util::Take, AsyncRead, AsyncReadAt, IoResult};

/// Shared code for read a scalar value from the underlying reader.
macro_rules! read_scalar {
    ($t:ty, $be:ident, $le:ident) => {
        ::paste::paste! {
            #[doc = concat!("Read a big endian `", stringify!($t), "` from the underlying reader.")]
            async fn [< read_ $t >](&mut self) -> IoResult<$t> {
                use ::compio_buf::{arrayvec::ArrayVec, BufResult};

                const LEN: usize = ::std::mem::size_of::<$t>();
                let BufResult(len, buf) = self.read_exact(ArrayVec::<u8, LEN>::new()).await;
                assert_eq!(len?, LEN, "read_exact returned unexpected length");
                // Safety: We just checked that the buffer is the correct size
                Ok($t::$be(unsafe { buf.into_inner_unchecked() }))
            }

            #[doc = concat!("Read a little endian `", stringify!($t), "` from the underlying reader.")]
            async fn [< read_ $t _le >](&mut self) -> IoResult<$t> {
                use ::compio_buf::{arrayvec::ArrayVec, BufResult};

                const LEN: usize = ::std::mem::size_of::<$t>();
                let BufResult(len, buf) = self.read_exact(ArrayVec::<u8, LEN>::new()).await;
                assert_eq!(len?, LEN, "read_exact returned unexpected length");
                // Safety: We just checked that the buffer is the correct size
                Ok($t::$le(unsafe { buf.into_inner_unchecked() }))
            }
        }
    };
}

/// Shared code for loop reading until reaching a certain length.
macro_rules! loop_read_exact {
    ($buf:ident,$len:expr,loop $read_expr:expr) => {
        loop_read_exact!($buf, $len, read, loop $read_expr)
    };
    ($buf:ident,$len:expr,$tracker:ident, loop $read_expr:expr) => {
        let mut $tracker = 0;
        let len = $len;

        while $tracker < len {
            ($tracker, $buf) = buf_try!($read_expr.await.and_then(|n, b| {
                if n == 0 {
                    use ::std::io::{Error, ErrorKind};
                    (Err(Error::new(ErrorKind::UnexpectedEof, "failed to fill whole buffer",)), b)
                } else {
                    (Ok($tracker + n), b)
                }
            }));
        }
        return BufResult(Ok($tracker), $buf)
    };
}

/// Implemented as an extension trait, adding utility methods to all
/// [`AsyncRead`] types. Callers will tend to import this trait instead of
/// [`AsyncRead`].
pub trait AsyncReadExt: AsyncRead {
    /// Creates a "by reference" adaptor for this instance of [`AsyncRead`].
    ///
    /// The returned adapter also implements [`AsyncRead`] and will simply
    /// borrow this current reader.
    fn by_ref(&mut self) -> &mut Self
    where
        Self: Sized,
    {
        self
    }

    /// Read the exact number of bytes required to fill the buf.
    async fn read_exact<T: IoBufMut>(&mut self, mut buf: T) -> BufResult<usize, T> {
        loop_read_exact!(buf, buf.buf_capacity() - buf.buf_len(), loop self.read(buf));
    }

    /// Read all bytes until underlying reader reaches `EOF`.
    async fn read_all(&mut self) -> IoResult<Vec<u8>> {
        let mut buf = Vec::<u8>::with_capacity(128);
        let mut n = 0;

        while n != 0 {
            (n, buf) = buf_try!(@try self.read(buf).await);
            if buf.len() == buf.capacity() {
                buf.reserve(buf.capacity());
            }
        }

        Ok(buf)
    }

    /// Read the exact number of bytes required to fill the vectored buf.
    async fn read_vectored_exact<T: IoVectoredBufMut>(&mut self, buf: T) -> BufResult<usize, T> {
        let mut iter = match buf.owned_iter_mut() {
            Ok(buf) => buf,
            Err(buf) => return BufResult(Ok(0), buf),
        };
        let mut total = 0;

        loop {
            let len = iter.uninit_len();
            if len == 0 {
                continue;
            }

            match self.read_exact(iter).await {
                BufResult(Ok(n), ret) => {
                    iter = ret;
                    total += n;
                }
                BufResult(Err(e), iter) => return BufResult(Err(e), iter.into_inner()),
            };

            match iter.next() {
                Ok(next) => iter = next,
                Err(buf) => return BufResult(Ok(total), buf),
            }
        }
    }

    /// Creates an adaptor which reads at most `limit` bytes from it.
    ///
    /// This function returns a new instance of `AsyncRead` which will read
    /// at most `limit` bytes, after which it will always return EOF
    /// (`Ok(0)`). Any read errors will not count towards the number of
    /// bytes read and future calls to [`read()`] may succeed.
    ///
    /// [`read()`]: AsyncRead::read
    fn take(self, limit: u64) -> Take<Self>
    where
        Self: Sized,
    {
        Take::new(self, limit)
    }

    read_scalar!(u8, from_be_bytes, from_le_bytes);
    read_scalar!(u16, from_be_bytes, from_le_bytes);
    read_scalar!(u32, from_be_bytes, from_le_bytes);
    read_scalar!(u64, from_be_bytes, from_le_bytes);
    read_scalar!(u128, from_be_bytes, from_le_bytes);
    read_scalar!(i8, from_be_bytes, from_le_bytes);
    read_scalar!(i16, from_be_bytes, from_le_bytes);
    read_scalar!(i32, from_be_bytes, from_le_bytes);
    read_scalar!(i64, from_be_bytes, from_le_bytes);
    read_scalar!(i128, from_be_bytes, from_le_bytes);
    read_scalar!(f32, from_be_bytes, from_le_bytes);
    read_scalar!(f64, from_be_bytes, from_le_bytes);
}

impl<A: AsyncRead + ?Sized> AsyncReadExt for A {}

/// Implemented as an extension trait, adding utility methods to all
/// [`AsyncReadAt`] types. Callers will tend to import this trait instead of
/// [`AsyncReadAt`].
pub trait AsyncReadAtExt: AsyncReadAt {
    /// Read the exact number of bytes required to fill `buffer`.
    ///
    /// This function reads as many bytes as necessary to completely fill the
    /// uninitialized space of specified `buffer`.
    ///
    /// # Errors
    ///
    /// If this function encounters an "end of file" before completely filling
    /// the buffer, it returns an error of the kind
    /// [`ErrorKind::UnexpectedEof`]. The contents of `buffer` are unspecified
    /// in this case.
    ///
    /// If any other read error is encountered then this function immediately
    /// returns. The contents of `buffer` are unspecified in this case.
    ///
    /// If this function returns an error, it is unspecified how many bytes it
    /// has read, but it will never read more than would be necessary to
    /// completely fill the buffer.
    ///
    /// [`ErrorKind::UnexpectedEof`]: std::io::ErrorKind::UnexpectedEof
    async fn read_exact_at<T: IoBufMut>(&self, mut buf: T, pos: u64) -> BufResult<usize, T> {
        loop_read_exact!(
            buf,
            buf.buf_capacity() - buf.buf_len(),
            read,
            loop self.read_at(buf, pos + read as u64)
        );
    }
}

impl<A: AsyncReadAt + ?Sized> AsyncReadAtExt for A {}
