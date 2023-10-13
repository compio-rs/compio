use compio_buf::{buf_try, BufResult, IoBuf, IoBufMut, IoVectoredBufMut};

use crate::{util::unfilled_err, AsyncRead, AsyncReadAt, IoResult};

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
macro_rules! loop_read {
    ($buf:ident,$len:expr,loop $read_expr:expr) => {
        loop_read!($buf, $len, read, loop $read_expr)
    };
    ($buf:ident,$len:expr,$tracker:ident, loop $read_expr:expr) => {
        let mut $tracker = 0;
        let len = $len;

        while $tracker < len {
            ($tracker, $buf) = buf_try!($read_expr.await.and_then(|n, mut b| {
                if n == 0 {
                    (unfilled_err!(), b)
                } else {
                    unsafe { b.set_buf_init(n) };
                    (Ok($tracker + n), b)
                }
            }));
        }
        return BufResult(Ok($tracker), $buf)
    };
}

pub trait AsyncReadExt: AsyncRead {
    /// Read the exact number of bytes required to fill the buf.
    async fn read_exact<T: IoBufMut>(&mut self, mut buf: T) -> BufResult<usize, T> {
        loop_read!(buf, buf.buf_capacity() - buf.buf_len(), loop self.read(buf));
    }

    /// Read the exact number of bytes required to fill the vector buf.
    async fn read_vectored_exact<T: IoVectoredBufMut>(&mut self, mut buf: T) -> BufResult<usize, T>
    where
        T::Item: IoBufMut,
    {
        loop_read!(
            buf,
            buf.buf_iter_mut()
                .map(|x| x.buf_capacity() - x.buf_len())
                .sum(),
            loop self.read_vectored(buf)
        );
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

pub trait AsyncReadAtExt: AsyncReadAt {
    async fn read_exact_at<T: IoBufMut>(&self, mut buf: T, pos: usize) -> BufResult<usize, T> {
        loop_read!(
            buf,
            buf.buf_capacity() - buf.buf_len(),
            read,
            loop self.read_at(buf, pos + read)
        );
    }
}

impl<A: AsyncReadAt> AsyncReadAtExt for A {}
