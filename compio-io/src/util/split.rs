use std::sync::Arc;

use compio_buf::{BufResult, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};
use futures_util::lock::Mutex;

use crate::{AsyncRead, AsyncWrite, IoResult};

/// Splits a single value implementing `AsyncRead + AsyncWrite` into separate
/// [`AsyncRead`] and [`AsyncWrite`] handles.
pub fn split<T: AsyncRead + AsyncWrite>(stream: T) -> (ReadHalf<T>, WriteHalf<T>) {
    let stream = Arc::new(Mutex::new(stream));
    (ReadHalf(stream.clone()), WriteHalf(stream))
}

/// The readable half of a value returned from [`split`].
#[derive(Debug)]
pub struct ReadHalf<T>(Arc<Mutex<T>>);

impl<T: Unpin> ReadHalf<T> {
    /// Reunites with a previously split [`WriteHalf`].
    ///
    /// # Panics
    ///
    /// If this [`ReadHalf`] and the given [`WriteHalf`] do not originate from
    /// the same [`split`] operation this method will panic.
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

/// The writable half of a value returned from [`split`].
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
