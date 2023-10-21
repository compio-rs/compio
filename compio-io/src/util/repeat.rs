use std::mem::MaybeUninit;

use compio_buf::BufResult;

use crate::{AsyncBufRead, AsyncRead, IoResult};

/// A reader that infinitely repeats one byte constructed via [`repeat`].
///
/// All reads from this reader will succeed by filling the specified buffer with
/// the given byte.
///
/// # Examples
///
/// ```rust
/// # #[compio_macros::main] async fn main() {
/// use compio_io::{self, AsyncRead, AsyncReadExt};
///
/// let (len, buffer) = compio_io::repeat(42)
///     .read(Vec::with_capacity(3))
///     .await
///     .unwrap();
///
/// assert_eq!(buffer.as_slice(), [42, 42, 42]);
/// assert_eq!(len, 3);
/// # }
/// ```
pub struct Repeat(u8);

impl AsyncRead for Repeat {
    async fn read<B: compio_buf::IoBufMut>(
        &mut self,
        mut buf: B,
    ) -> compio_buf::BufResult<usize, B> {
        let slice = buf.as_mut_slice();

        let len = slice.len();
        slice.fill(MaybeUninit::new(self.0));
        unsafe { buf.set_buf_init(len) };

        BufResult(Ok(len), buf)
    }
}

impl AsyncBufRead for Repeat {
    async fn fill_buf(&mut self) -> IoResult<&'_ [u8]> {
        Ok(std::slice::from_ref(&self.0))
    }

    fn consume(&mut self, _: usize) {}
}

/// Creates a reader that infinitely repeats one byte.
///
/// All reads from this reader will succeed by filling the specified buffer with
/// the given byte.
///
/// # Examples
///
/// ```rust
/// # #[compio_macros::main] async fn main() {
/// use compio_io::{self, AsyncRead, AsyncReadExt};
///
/// let (len, buffer) = compio_io::repeat(42)
///     .read_exact(Vec::with_capacity(3))
///     .await
///     .unwrap();
///
/// assert_eq!(buffer.as_slice(), [42, 42, 42]);
/// assert_eq!(len, 3);
/// # }
/// ```
pub fn repeat(byte: u8) -> Repeat {
    Repeat(byte)
}
