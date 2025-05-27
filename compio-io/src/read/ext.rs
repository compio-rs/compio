#[cfg(feature = "allocator_api")]
use std::alloc::Allocator;
use std::io::ErrorKind;

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut, IoVectoredBufMut, t_alloc};

use crate::{AsyncRead, AsyncReadAt, IoResult, util::Take};

/// Shared code for read a scalar value from the underlying reader.
macro_rules! read_scalar {
    ($t:ty, $be:ident, $le:ident) => {
        ::paste::paste! {
            #[doc = concat!("Read a big endian `", stringify!($t), "` from the underlying reader.")]
            async fn [< read_ $t >](&mut self) -> IoResult<$t> {
                use ::compio_buf::{arrayvec::ArrayVec, BufResult};

                const LEN: usize = ::std::mem::size_of::<$t>();
                let BufResult(res, buf) = self.read_exact(ArrayVec::<u8, LEN>::new()).await;
                res?;
                // Safety: We just checked that the buffer is the correct size
                Ok($t::$be(unsafe { buf.into_inner_unchecked() }))
            }

            #[doc = concat!("Read a little endian `", stringify!($t), "` from the underlying reader.")]
            async fn [< read_ $t _le >](&mut self) -> IoResult<$t> {
                use ::compio_buf::{arrayvec::ArrayVec, BufResult};

                const LEN: usize = ::std::mem::size_of::<$t>();
                let BufResult(res, buf) = self.read_exact(ArrayVec::<u8, LEN>::new()).await;
                res?;
                // Safety: We just checked that the buffer is the correct size
                Ok($t::$le(unsafe { buf.into_inner_unchecked() }))
            }
        }
    };
}

/// Shared code for loop reading until reaching a certain length.
macro_rules! loop_read_exact {
    ($buf:ident, $len:expr, $tracker:ident,loop $read_expr:expr) => {
        let mut $tracker = 0;
        let len = $len;

        while $tracker < len {
            match $read_expr.await.into_inner() {
                BufResult(Ok(0), buf) => {
                    return BufResult(
                        Err(::std::io::Error::new(
                            ::std::io::ErrorKind::UnexpectedEof,
                            "failed to fill whole buffer",
                        )),
                        buf,
                    );
                }
                BufResult(Ok(n), buf) => {
                    $tracker += n;
                    $buf = buf;
                }
                BufResult(Err(ref e), buf) if e.kind() == ::std::io::ErrorKind::Interrupted => {
                    $buf = buf;
                }
                BufResult(Err(e), buf) => return BufResult(Err(e), buf),
            }
        }
        return BufResult(Ok(()), $buf)
    };
}

macro_rules! loop_read_vectored {
    ($buf:ident, $tracker:ident : $tracker_ty:ty, $iter:ident,loop $read_expr:expr) => {{
        use ::compio_buf::OwnedIterator;

        let mut $iter = match $buf.owned_iter() {
            Ok(buf) => buf,
            Err(buf) => return BufResult(Ok(()), buf),
        };
        let mut $tracker: $tracker_ty = 0;

        loop {
            let len = $iter.buf_capacity();
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
            let len = $iter.buf_capacity();
            if len > 0 {
                return $read_expr.await.into_inner();
            }

            match $iter.next() {
                Ok(next) => $iter = next,
                Err(buf) => return BufResult(Ok(0), buf),
            }
        }
    }};
}

macro_rules! loop_read_to_end {
    ($buf:ident, $tracker:ident : $tracker_ty:ty,loop $read_expr:expr) => {{
        let mut $tracker: $tracker_ty = 0;
        loop {
            if $buf.len() == $buf.capacity() {
                $buf.reserve(32);
            }
            match $read_expr.await.into_inner() {
                BufResult(Ok(0), buf) => {
                    $buf = buf;
                    break;
                }
                BufResult(Ok(read), buf) => {
                    $tracker += read as $tracker_ty;
                    $buf = buf;
                }
                BufResult(Err(ref e), buf) if e.kind() == ::std::io::ErrorKind::Interrupted => {
                    $buf = buf
                }
                res => return res,
            }
        }
        BufResult(Ok($tracker as usize), $buf)
    }};
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
    async fn read_exact<T: IoBufMut>(&mut self, mut buf: T) -> BufResult<(), T> {
        loop_read_exact!(buf, buf.buf_capacity(), read, loop self.read(buf.slice(read..)));
    }

    /// Read all bytes as [`String`] until underlying reader reaches `EOF`.
    async fn read_to_string(&mut self, buf: String) -> BufResult<usize, String> {
        let BufResult(res, buf) = self.read_to_end(buf.into_bytes()).await;
        match res {
            Err(err) => BufResult(Err(err), String::new()),
            Ok(n) => match String::from_utf8(buf) {
                Err(err) => BufResult(
                    Err(std::io::Error::new(ErrorKind::InvalidData, err)),
                    String::new(),
                ),
                Ok(data) => BufResult(Ok(n), data),
            },
        }
    }

    /// Read all bytes until underlying reader reaches `EOF`.
    async fn read_to_end<#[cfg(feature = "allocator_api")] A: Allocator + 'static>(
        &mut self,
        mut buf: t_alloc!(Vec, u8, A),
    ) -> BufResult<usize, t_alloc!(Vec, u8, A)> {
        loop_read_to_end!(buf, total: usize, loop self.read(buf.slice(total..)))
    }

    /// Read the exact number of bytes required to fill the vectored buf.
    async fn read_vectored_exact<T: IoVectoredBufMut>(&mut self, buf: T) -> BufResult<(), T> {
        loop_read_vectored!(buf, _total: usize, iter, loop self.read_exact(iter))
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
    async fn read_exact_at<T: IoBufMut>(&self, mut buf: T, pos: u64) -> BufResult<(), T> {
        loop_read_exact!(
            buf,
            buf.buf_capacity(),
            read,
            loop self.read_at(buf.slice(read..), pos + read as u64)
        );
    }

    /// Read all bytes until EOF in this source, placing them into `buffer`.
    ///
    /// All bytes read from this source will be appended to the specified buffer
    /// `buffer`. This function will continuously call [`read_at()`] to append
    /// more data to `buffer` until [`read_at()`] returns [`Ok(0)`].
    ///
    /// If successful, this function will return the total number of bytes read.
    ///
    /// [`read_at()`]: AsyncReadAt::read_at
    async fn read_to_end_at<#[cfg(feature = "allocator_api")] A: Allocator + 'static>(
        &self,
        mut buffer: t_alloc!(Vec, u8, A),
        pos: u64,
    ) -> BufResult<usize, t_alloc!(Vec, u8, A)> {
        loop_read_to_end!(buffer, total: u64, loop self.read_at(buffer.slice(total as usize..), pos + total))
    }

    /// Like [`AsyncReadExt::read_vectored_exact`], expect that it reads at a
    /// specified position.
    async fn read_vectored_exact_at<T: IoVectoredBufMut>(
        &self,
        buf: T,
        pos: u64,
    ) -> BufResult<(), T> {
        loop_read_vectored!(buf, total: u64, iter, loop self.read_exact_at(iter, pos + total))
    }
}

impl<A: AsyncReadAt + ?Sized> AsyncReadAtExt for A {}
