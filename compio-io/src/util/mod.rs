//! IO related utilities functions for ease of use.

mod take;
pub use take::Take;

mod null;
pub use null::{Null, null};

mod repeat;
pub use repeat::{Repeat, repeat};

mod internal;
pub(crate) use internal::*;

use crate::{AsyncRead, AsyncWrite, AsyncWriteExt, IoResult};

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
pub async fn copy<R: AsyncRead, W: AsyncWrite>(reader: &mut R, writer: &mut W) -> IoResult<u64> {
    let mut buf = Vec::with_capacity(DEFAULT_BUF_SIZE);
    let mut total = 0u64;

    loop {
        let res;
        (res, buf) = reader.read(buf).await.into();
        match res {
            Ok(0) => break,
            Ok(read) => {
                total += read as u64;
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {
                continue;
            }
            Err(e) => return Err(e),
        }
        let res;
        (res, buf) = writer.write_all(buf).await.into();
        res?;
    }

    Ok(total)
}
