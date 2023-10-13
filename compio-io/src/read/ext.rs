use crate::{AsyncRead, IoResult};

macro_rules! read_fn {
    ($t:ty) => {};
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

pub trait AsyncReadExt: AsyncRead {
    read_fn!(u8, from_be_bytes, from_le_bytes);
    read_fn!(u16, from_be_bytes, from_le_bytes);
    read_fn!(u32, from_be_bytes, from_le_bytes);
    read_fn!(u64, from_be_bytes, from_le_bytes);
    read_fn!(u128, from_be_bytes, from_le_bytes);
    read_fn!(i8, from_be_bytes, from_le_bytes);
    read_fn!(i16, from_be_bytes, from_le_bytes);
    read_fn!(i32, from_be_bytes, from_le_bytes);
    read_fn!(i64, from_be_bytes, from_le_bytes);
    read_fn!(i128, from_be_bytes, from_le_bytes);
    read_fn!(f32, from_be_bytes, from_le_bytes);
    read_fn!(f64, from_be_bytes, from_le_bytes);
}
