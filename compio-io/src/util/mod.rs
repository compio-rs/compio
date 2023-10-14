//! IO related utilities functions for ease of use.

mod take;
pub use take::Take;

mod null;
pub use null::{null, Null};

mod repeat;
pub use repeat::{repeat, Repeat};

mod internal;
pub(crate) use internal::*;

use crate::{buffer::Buffer, AsyncRead, AsyncWrite, AsyncWriteExt, IoResult};

/// Asynchronously copies the entire contents of a reader into a writer.
///
/// This function returns a future that will continuously read data from
/// `reader` and then write it into `writer` in a streaming fashion until
/// `reader` returns EOF or fails.
///
/// On success, the total number of bytes that were copied from `reader` to
/// `writer` is returned.
///
/// This is an asynchronous version of [`std::io::copy`][std].
///
/// A heap-allocated copy buffer with 8 KB is created to take data from the
/// reader to the writer.
pub async fn copy<'a, R: AsyncRead, W: AsyncWrite>(
    reader: &'a mut R,
    writer: &'a mut W,
) -> IoResult<usize> {
    let mut buf = Buffer::with_capacity(DEFAULT_BUF_SIZE);
    let mut total = 0;

    loop {
        let read = buf.with(|w| reader.read(w)).await?;

        // When EOF is reached, we are terminating, so flush before that
        if read == 0 || buf.need_flush() {
            let written = buf.with(|w| writer.write_all(w)).await?;
            if written == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "0 byte was written into the writer",
                ));
            }
            total += written;

            if buf.advance(written) {
                buf.reset()
            }
        }

        if read == 0 {
            writer.flush().await?;
            break;
        }
    }

    Ok(total)
}
