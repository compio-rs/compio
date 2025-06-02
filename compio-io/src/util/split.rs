//! Functionality to split an I/O type into separate read and write halves.

use std::{fmt::Debug, sync::Arc};

use compio_buf::{BufResult, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};
use futures_util::lock::Mutex;

use crate::{AsyncRead, AsyncReadAt, AsyncWrite, AsyncWriteAt, IoResult, util::bilock::BiLock};

/// Splits a single value implementing `AsyncRead + AsyncWrite` into separate
/// [`AsyncRead`] and [`AsyncWrite`] handles.with internal synchronization.
pub fn split<T: AsyncRead + AsyncWrite>(stream: T) -> (ReadHalf<T>, WriteHalf<T>) {
    Split::new(stream).split()
}

/// Splits a single value implementing `AsyncRead + AsyncWrite` into separate
/// [`AsyncRead`] and [`AsyncWrite`] handles without internal synchronization
/// (not `Send` and `Sync`).
pub fn split_unsync<T: AsyncRead + AsyncWrite>(
    stream: T,
) -> (UnsyncReadHalf<T>, UnsyncWriteHalf<T>) {
    UnsyncSplit::new(stream).split()
}

/// A trait for types that can be split into separate read and write halves.
///
/// This trait enables an I/O type to be divided into two separate components:
/// one for reading and one for writing. This is particularly useful in async
/// contexts where you might want to perform concurrent read and write
/// operations from different tasks.
///
/// # Implementor
/// - Any `(R, W)` tuple implements this trait.
/// - `TcpStream`, `UnixStream` and references to them in `compio::net`
///   implement this trait without any lock thanks to the underlying sockets'
///   duplex nature.
/// - `File` and named pipes in `compio::fs` implement this trait with
///   [`ReadHalf`] and [`WriteHalf`] being the file itself since it's
///   reference-counted under the hood.
/// - For other type to be compatible with this trait, it must be wrapped with
///   [`UnsyncSplit`] or [`Split`], which wrap the type in a unsynced or synced
///   lock respectively.
pub trait Splittable {
    /// The type of the read half, which normally implements [`AsyncRead`] or
    /// [`AsyncReadAt`].
    type ReadHalf;

    /// The type of the write half, which normally implements [`AsyncWrite`] or
    /// [`AsyncWriteAt`].
    type WriteHalf;

    /// Consumes `self` and returns a tuple containing separate read and write
    /// halves.
    ///
    /// The returned halves can be used independently to perform read and write
    /// operations respectively, potentially from different tasks
    /// concurrently.
    fn split(self) -> (Self::ReadHalf, Self::WriteHalf);
}

/// Enables splitting an I/O type into separate read and write halves
/// without requiring thread-safety.
///
/// # Examples
///
/// ```
/// use compio::io::util::UnsyncSplit;
///
/// // Create a splittable stream
/// let stream = /* some stream */;
/// let unsync = UnsyncSplit::new(stream);
/// let (read_half, write_half) = unsync.split();
/// ```
#[derive(Debug)]
pub struct UnsyncSplit<T>(BiLock<T>, BiLock<T>);

impl<T> UnsyncSplit<T> {
    /// Creates a new `UnsyncSplit` from the given stream.
    pub fn new(stream: T) -> Self {
        let (r, w) = BiLock::new(stream);
        UnsyncSplit(r, w)
    }
}

impl<T> Splittable for UnsyncSplit<T> {
    type ReadHalf = UnsyncReadHalf<T>;
    type WriteHalf = UnsyncWriteHalf<T>;

    fn split(self) -> (Self::ReadHalf, Self::WriteHalf) {
        (UnsyncReadHalf(self.0), UnsyncWriteHalf(self.1))
    }
}

impl<R, W> Splittable for (R, W) {
    type ReadHalf = R;
    type WriteHalf = W;

    fn split(self) -> (Self::ReadHalf, Self::WriteHalf) {
        self
    }
}

/// The readable half of a value returned from [`split`].
#[derive(Debug)]
pub struct UnsyncReadHalf<T>(BiLock<T>);

impl<T> UnsyncReadHalf<T> {
    /// Reunites with a previously split [`UnsyncWriteHalf`].
    ///
    /// # Panics
    ///
    /// If this [`UnsyncReadHalf`] and the given [`UnsyncWriteHalf`] do not
    /// originate from the same [`split_unsync`](super::split_unsync) operation
    /// this method will panic.
    #[track_caller]
    pub fn unsplit(self, other: UnsyncWriteHalf<T>) -> T {
        self.0.try_join(other.0).expect(
            "`UnsyncReadHalf` and `UnsyncWriteHalf` must originate from the same `UnsyncSplit`",
        )
    }
}

impl<T: AsyncRead> AsyncRead for UnsyncReadHalf<T> {
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        self.0.lock().await.read(buf).await
    }

    async fn read_vectored<V: IoVectoredBufMut>(&mut self, buf: V) -> BufResult<usize, V> {
        self.0.lock().await.read_vectored(buf).await
    }
}

impl<T: AsyncReadAt> AsyncReadAt for UnsyncReadHalf<T> {
    async fn read_at<B: IoBufMut>(&self, buf: B, pos: u64) -> BufResult<usize, B> {
        self.0.lock().await.read_at(buf, pos).await
    }
}

/// The writable half of a value returned from [`split`](super::split).
#[derive(Debug)]
pub struct UnsyncWriteHalf<T>(BiLock<T>);

impl<T: AsyncWrite> AsyncWrite for UnsyncWriteHalf<T> {
    async fn write<B: IoBuf>(&mut self, buf: B) -> BufResult<usize, B> {
        self.0.lock().await.write(buf).await
    }

    async fn write_vectored<B: IoVectoredBuf>(&mut self, buf: B) -> BufResult<usize, B> {
        self.0.lock().await.write_vectored(buf).await
    }

    async fn flush(&mut self) -> IoResult<()> {
        self.0.lock().await.flush().await
    }

    async fn shutdown(&mut self) -> IoResult<()> {
        self.0.lock().await.shutdown().await
    }
}

impl<T: AsyncWriteAt> AsyncWriteAt for UnsyncWriteHalf<T> {
    async fn write_at<B: IoBuf>(&mut self, buf: B, pos: u64) -> BufResult<usize, B> {
        self.0.lock().await.write_at(buf, pos).await
    }

    async fn write_vectored_at<B: IoVectoredBuf>(
        &mut self,
        buf: B,
        pos: u64,
    ) -> BufResult<usize, B> {
        self.0.lock().await.write_vectored_at(buf, pos).await
    }
}

/// Splitting an I/O type into separate read and write halves
#[derive(Debug)]
pub struct Split<T>(Arc<Mutex<T>>);

impl<T> Split<T> {
    /// Creates a new `Split` from the given stream.
    pub fn new(stream: T) -> Self {
        Split(Arc::new(Mutex::new(stream)))
    }
}

impl<T: AsyncRead + AsyncWrite> Splittable for Split<T> {
    type ReadHalf = ReadHalf<T>;
    type WriteHalf = WriteHalf<T>;

    fn split(self) -> (Self::ReadHalf, Self::WriteHalf) {
        (ReadHalf(self.0.clone()), WriteHalf(self.0))
    }
}

/// The readable half of a value returned from [`split`](super::split).
#[derive(Debug)]
pub struct ReadHalf<T>(Arc<Mutex<T>>);

impl<T: Unpin> ReadHalf<T> {
    /// Reunites with a previously split [`WriteHalf`].
    ///
    /// # Panics
    ///
    /// If this [`ReadHalf`] and the given [`WriteHalf`] do not originate from
    /// the same [`split`](super::split) operation this method will panic.
    /// This can be checked ahead of time by comparing the stored pointer
    /// of the two halves.
    #[track_caller]
    pub fn unsplit(self, w: WriteHalf<T>) -> T {
        if Arc::ptr_eq(&self.0, &w.0) {
            drop(w);
            let inner = Arc::try_unwrap(self.0).expect("`Arc::try_unwrap` failed");
            inner.into_inner()
        } else {
            #[cold]
            fn panic_unrelated() -> ! {
                panic!("Unrelated `WriteHalf` passed to `ReadHalf::unsplit`.")
            }

            panic_unrelated()
        }
    }
}

impl<T: AsyncRead> AsyncRead for ReadHalf<T> {
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        self.0.lock().await.read(buf).await
    }

    async fn read_vectored<V: IoVectoredBufMut>(&mut self, buf: V) -> BufResult<usize, V> {
        self.0.lock().await.read_vectored(buf).await
    }
}

impl<T: AsyncReadAt> AsyncReadAt for ReadHalf<T> {
    async fn read_at<B: IoBufMut>(&self, buf: B, pos: u64) -> BufResult<usize, B> {
        self.0.lock().await.read_at(buf, pos).await
    }
}

/// The writable half of a value returned from [`split`](super::split).
#[derive(Debug)]
pub struct WriteHalf<T>(Arc<Mutex<T>>);

impl<T: AsyncWrite> AsyncWrite for WriteHalf<T> {
    async fn write<B: IoBuf>(&mut self, buf: B) -> BufResult<usize, B> {
        self.0.lock().await.write(buf).await
    }

    async fn write_vectored<B: IoVectoredBuf>(&mut self, buf: B) -> BufResult<usize, B> {
        self.0.lock().await.write_vectored(buf).await
    }

    async fn flush(&mut self) -> IoResult<()> {
        self.0.lock().await.flush().await
    }

    async fn shutdown(&mut self) -> IoResult<()> {
        self.0.lock().await.shutdown().await
    }
}

impl<T: AsyncWriteAt> AsyncWriteAt for WriteHalf<T> {
    async fn write_at<B: IoBuf>(&mut self, buf: B, pos: u64) -> BufResult<usize, B> {
        self.0.lock().await.write_at(buf, pos).await
    }

    async fn write_vectored_at<B: IoVectoredBuf>(
        &mut self,
        buf: B,
        pos: u64,
    ) -> BufResult<usize, B> {
        self.0.lock().await.write_vectored_at(buf, pos).await
    }
}
