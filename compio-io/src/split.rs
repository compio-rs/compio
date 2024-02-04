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
pub struct ReadHalf<T>(Arc<Mutex<T>>);

impl<T: AsyncRead> AsyncRead for ReadHalf<T> {
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        self.0.lock().await.read(buf).await
    }

    async fn read_vectored<V: IoVectoredBufMut>(&mut self, buf: V) -> BufResult<usize, V> {
        self.0.lock().await.read_vectored(buf).await
    }
}

/// The writable half of a value returned from [`split`].
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
