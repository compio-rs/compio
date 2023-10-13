use compio_buf::{buf_try, BufResult, IntoInner, IoBuf};

use crate::{AsyncWrite, AsyncWriteAt, IoResult};

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

pub trait AsyncWriteExt: AsyncWrite {
    /// Write the entire contents of a buffer into this writer.
    async fn write_all<T: IoBuf>(&mut self, mut buffer: T) -> BufResult<usize, T> {
        let buf_len = buffer.buf_len();
        let mut total_written = 0;
        while total_written < buf_len {
            let written;
            (written, buffer) =
                buf_try!(self.write(buffer.slice(total_written..)).await.into_inner());
            total_written += written;
        }
        BufResult(Ok(total_written), buffer)
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
