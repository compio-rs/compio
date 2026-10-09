use futures_util::future::join;

use crate::{AsyncRead, AsyncWrite, AsyncWriteExt, IoResult, util::Splittable};

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
/// On Linux with io_uring, unbuffered descriptor-backed streams can copy via
/// a kernel pipe without moving payload data through userspace. Other readers
/// and writers (including buffered and transforming adapters, file cursors,
/// and seekable `AsyncFd` handles) use an 8 KiB heap-allocated buffer.
/// Unsupported splice operations fall back to buffered copying without
/// discarding bytes already read.
///
/// The splice path preserves the kernel's default capacity for its private
/// pipe and batches eight fill/drain pairs without changing descriptor blocking
/// flags. Individual splice requests may be reduced to leave accounting
/// headroom in a destination socket's send buffer, without resizing the pipe
/// or changing socket options. [`copy_with_size`] controls transfer sizing,
/// while [`copy_buffered`] opts out of kernel-assisted copying.
///
/// At EOF the writer is flushed and shut down. Like buffered copying, this
/// operation is not cancellation-safe: bytes read but not yet written can be
/// lost when the future is dropped.
pub async fn copy<R: AsyncRead + ?Sized, W: AsyncWrite + ?Sized>(
    reader: &mut R,
    writer: &mut W,
) -> IoResult<u64> {
    reader.copy_to(writer, None).await
}

/// Asynchronously copies the entire contents of a reader into a writer with
/// a specified transfer size.
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
/// Like [`copy`], this allows kernel-assisted copying. `buf_size` sets the
/// userspace buffer size on the buffered path, or the requested pipe capacity
/// and maximum bytes per splice operation on the Linux io_uring path.
/// A destination socket's send-buffer budget may lower this maximum without
/// changing the requested pipe capacity.
/// Linux may round pipe capacity upward; if the requested capacity cannot be
/// obtained, copying falls back to a userspace buffer of `buf_size` bytes.
/// Sizes below the kernel's default pipe capacity stay on the buffered path:
/// small-pipe splicing is slower than an equally sized userspace buffer.
///
/// When kernel-assisted copying is unavailable or a splice operation is
/// unsupported, this automatically falls back to buffered copying with
/// `buf_size` bytes, preserving any bytes already read into the kernel pipe.
/// Use [`copy_buffered`] to force userspace copying without attempting the
/// kernel-assisted path.
pub async fn copy_with_size<R: AsyncRead + ?Sized, W: AsyncWrite + ?Sized>(
    reader: &mut R,
    writer: &mut W,
    buf_size: usize,
) -> IoResult<u64> {
    reader.copy_to(writer, Some(buf_size)).await
}

/// Asynchronously copies a reader into a writer using a userspace buffer.
///
/// This always uses a heap-allocated buffer of `buf_size` bytes, even when the
/// endpoints support kernel-assisted copying. At EOF the writer is flushed and
/// shut down, and the total number of copied bytes is returned.
///
/// This is also the buffered fallback used by [`copy`] and [`copy_with_size`]
/// when kernel-assisted copying is unavailable or unsupported. [`copy`] uses
/// an 8 KiB buffer; [`copy_with_size`] uses its requested `buf_size`. Calling
/// this function directly skips the kernel-assisted path entirely.
///
/// Like [`copy`], this operation is not cancellation-safe: bytes already read
/// but not yet written can be lost when the future is dropped.
pub async fn copy_buffered<R: AsyncRead + ?Sized, W: AsyncWrite + ?Sized>(
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
/// transfer size for each direction. Each direction uses [`copy_with_size`],
/// including kernel-assisted copying when eligible.
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
