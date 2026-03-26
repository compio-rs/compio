use futures_util::future::join;

use crate::{
    AsyncRead, AsyncWrite, AsyncWriteExt, IoResult,
    util::{DEFAULT_BUF_SIZE, Splittable},
};

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
/// A heap-allocated copy buffer with 8 KiB is created to take data from the
/// reader to the writer.
pub async fn copy<R: AsyncRead, W: AsyncWrite>(reader: &mut R, writer: &mut W) -> IoResult<u64> {
    copy_with_size(reader, writer, DEFAULT_BUF_SIZE).await
}

/// Asynchronously copies the entire contents of a reader into a writer with
/// specified buffer sizes.
///
/// This function returns a future that will continuously read data from
/// `reader` and then write it into `writer` in a streaming fashion until
/// `reader` returns EOF or fails.
///
/// On success, the total number of bytes that were copied from `reader` to
/// `writer` is returned.
///
/// This is an asynchronous version of [`std::io::copy`][std].
pub async fn copy_with_size<R: AsyncRead, W: AsyncWrite>(
    reader: &mut R,
    writer: &mut W,
    buf_size: usize,
) -> IoResult<u64> {
    let mut buf = Vec::with_capacity(buf_size);
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
        buf.clear();
    }

    writer.flush().await?;
    writer.shutdown().await?;

    Ok(total)
}

/// Asynchronously copies data bidirectionally between two pairs of reader and
/// writer.
///
/// This function takes two `Splittable` objects, `reader` and `writer`, and
/// splits them into their respective read and write halves. It then
/// concurrently copies data from the read half of `reader` to the write half of
/// `writer`, and from the read half of `writer` to the write half of `reader`.
/// The function returns a tuple containing the results of both copy operations,
/// which indicate the total number of bytes copied in each direction or any
/// errors that occurred during the copying process.
pub async fn copy_bidirectional<A, B>(reader: A, writer: B) -> (IoResult<u64>, IoResult<u64>)
where
    A: Splittable<ReadHalf: AsyncRead, WriteHalf: AsyncWrite>,
    B: Splittable<ReadHalf: AsyncRead, WriteHalf: AsyncWrite>,
{
    let (mut ar, mut aw) = reader.split();
    let (mut br, mut bw) = writer.split();

    join(copy(&mut ar, &mut bw), copy(&mut br, &mut aw)).await
}

/// Asynchronously copies data bidirectionally between two pairs of reader and
/// writer with specified buffer sizes.
///
/// This function is like `copy_bidirectional`, but allows you to specify the
/// buffer sizes for each direction of copying.
pub async fn copy_bidirectional_with_sizes<A, B>(
    reader: A,
    writer: B,
    a_to_b_size: usize,
    b_to_a_size: usize,
) -> (IoResult<u64>, IoResult<u64>)
where
    A: Splittable<ReadHalf: AsyncRead, WriteHalf: AsyncWrite>,
    B: Splittable<ReadHalf: AsyncRead, WriteHalf: AsyncWrite>,
{
    let (mut ar, mut aw) = reader.split();
    let (mut br, mut bw) = writer.split();

    join(
        copy_with_size(&mut ar, &mut bw, a_to_b_size),
        copy_with_size(&mut br, &mut aw, b_to_a_size),
    )
    .await
}
