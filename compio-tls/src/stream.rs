use std::{
    borrow::Cow,
    io,
    mem::MaybeUninit,
    pin::Pin,
    task::{Context, Poll},
};

use compio_buf::{BufResult, IoBuf, IoBufMut, IoVectoredBuf};
use compio_io::{AsyncRead, AsyncWrite, compat::AsyncStream};

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
enum TlsStreamInner<S: Splittable> {
    #[cfg(feature = "native-tls")]
    NativeTls(crate::native::TlsStream<Pin<Box<AsyncStream<S>>>>),
    #[cfg(feature = "rustls")]
    Rustls(futures_rustls::TlsStream<Pin<Box<AsyncStream<S>>>>),
    #[cfg(feature = "py-dynamic-openssl")]
    PyDynamicOpenSsl(crate::py_ossl::TlsStream<Pin<Box<AsyncStream<S>>>>),
    #[cfg(not(any(
        feature = "native-tls",
        feature = "rustls",
        feature = "py-dynamic-openssl",
    )))]
    None(std::convert::Infallible, std::marker::PhantomData<S>),
}

impl<S: AsyncRead + AsyncWrite + Unpin + 'static> TlsStreamInner<S>
where
    for<'a> &'a S: AsyncRead + AsyncWrite,
{
    pub fn negotiated_alpn(&self) -> Option<Cow<'_, [u8]>> {
        match self {
            #[cfg(feature = "native-tls")]
            Self::NativeTls(s) => s.negotiated_alpn().ok().flatten().map(Cow::from),
            #[cfg(feature = "rustls")]
            Self::Rustls(s) => s.get_ref().1.alpn_protocol().map(Cow::from),
            #[cfg(feature = "py-dynamic-openssl")]
            Self::PyDynamicOpenSsl(s) => s.negotiated_alpn().map(Cow::from),
            #[cfg(not(any(
                feature = "native-tls",
                feature = "rustls",
                feature = "py-dynamic-openssl",
            )))]
            Self::None(f, ..) => match *f {},
        }
    }
}

/// A wrapper around an underlying raw stream which implements the TLS or SSL
/// protocol.
///
/// A `TlsStream<S>` represents a handshake that has been completed successfully
/// and both the server and the client are ready for receiving and sending
/// data. Bytes read from a `TlsStream` are decrypted from `S` and bytes written
/// to a `TlsStream` are encrypted when passing through to `S`.
#[derive(Debug)]
pub struct TlsStream<S: Splittable>(TlsStreamInner<S>);

impl<S: AsyncRead + AsyncWrite + Unpin + 'static> TlsStream<S>
where
    for<'a> &'a S: AsyncRead + AsyncWrite,
{
    /// Returns the negotiated ALPN protocol.
    pub fn negotiated_alpn(&self) -> Option<Cow<'_, [u8]>> {
        self.0.negotiated_alpn()
    }
}

#[cfg(feature = "native-tls")]
#[doc(hidden)]
impl<S> From<crate::native::TlsStream<Pin<Box<AsyncStream<S>>>>> for TlsStream<S> {
    fn from(value: crate::native::TlsStream<Pin<Box<AsyncStream<S>>>>) -> Self {
        Self(TlsStreamInner::NativeTls(value))
    }
}

#[cfg(feature = "rustls")]
#[doc(hidden)]
impl<S: Splittable> From<futures_rustls::client::TlsStream<Pin<Box<AsyncStream<S>>>>>
    for TlsStream<S>
{
    fn from(value: futures_rustls::client::TlsStream<Pin<Box<AsyncStream<S>>>>) -> Self {
        Self(TlsStreamInner::Rustls(futures_rustls::TlsStream::Client(
            value,
        )))
    }
}

#[cfg(feature = "rustls")]
#[doc(hidden)]
impl<S: Splittable> From<futures_rustls::server::TlsStream<Pin<Box<AsyncStream<S>>>>>
    for TlsStream<S>
{
    fn from(value: futures_rustls::server::TlsStream<Pin<Box<AsyncStream<S>>>>) -> Self {
        Self(TlsStreamInner::Rustls(futures_rustls::TlsStream::Server(
            value,
        )))
    }
}

#[cfg(feature = "py-dynamic-openssl")]
#[doc(hidden)]
impl<S> From<crate::py_ossl::TlsStream<Pin<Box<AsyncStream<S>>>>> for TlsStream<S> {
    fn from(value: crate::py_ossl::TlsStream<Pin<Box<AsyncStream<S>>>>) -> Self {
        Self(TlsStreamInner::PyDynamicOpenSsl(value))
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin + 'static> futures_util::AsyncRead for TlsStream<S>
where
    for<'a> &'a S: AsyncRead + AsyncWrite,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        match &mut self.get_mut().0 {
            #[cfg(feature = "native-tls")]
            TlsStreamInner::NativeTls(s) => Pin::new(s).poll_read(cx, buf),
            #[cfg(feature = "rustls")]
            TlsStreamInner::Rustls(s) => Pin::new(s).poll_read(cx, buf),
            #[cfg(feature = "py-dynamic-openssl")]
            TlsStreamInner::PyDynamicOpenSsl(s) => Pin::new(s).poll_read(cx, buf),
            #[cfg(not(any(
                feature = "native-tls",
                feature = "rustls",
                feature = "py-dynamic-openssl",
            )))]
            TlsStreamInner::None(f, ..) => match *f {},
        }
    }
}

impl<S: AsyncRead + AsyncWrite + Splittable + 'static> AsyncRead for TlsStream<S>
where
    S::ReadHalf: AsyncRead + Unpin,
    S::WriteHalf: AsyncWrite + Unpin,
{
    async fn read<B: IoBufMut>(&mut self, mut buf: B) -> BufResult<usize, B> {
        let slice = buf.as_uninit();
        slice.fill(MaybeUninit::new(0));
        // SAFETY: The memory has been initialized
        let slice =
            unsafe { std::slice::from_raw_parts_mut::<u8>(slice.as_mut_ptr().cast(), slice.len()) };
        let res = futures_util::AsyncReadExt::read(self, slice).await;
        let res = match res {
            Ok(len) => {
                unsafe { buf.advance_to(len) };
                Ok(len)
            }
            // TLS streams may return UnexpectedEof when the connection is closed.
            // https://docs.rs/rustls/latest/rustls/manual/_03_howto/index.html#unexpected-eof
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Ok(0),
            _ => res,
        };
        BufResult(res, buf)
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin + 'static> futures_util::AsyncWrite for TlsStream<S>
where
    for<'a> &'a S: AsyncRead + AsyncWrite,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match &mut self.get_mut().0 {
            #[cfg(feature = "native-tls")]
            TlsStreamInner::NativeTls(s) => Pin::new(s).poll_write(cx, buf),
            #[cfg(feature = "rustls")]
            TlsStreamInner::Rustls(s) => Pin::new(s).poll_write(cx, buf),
            #[cfg(feature = "py-dynamic-openssl")]
            TlsStreamInner::PyDynamicOpenSsl(s) => Pin::new(s).poll_write(cx, buf),
            #[cfg(not(any(
                feature = "native-tls",
                feature = "rustls",
                feature = "py-dynamic-openssl",
            )))]
            TlsStreamInner::None(f, ..) => match *f {},
        }
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        match &mut self.get_mut().0 {
            #[cfg(feature = "native-tls")]
            TlsStreamInner::NativeTls(s) => Pin::new(s).poll_write_vectored(cx, bufs),
            #[cfg(feature = "rustls")]
            TlsStreamInner::Rustls(s) => Pin::new(s).poll_write_vectored(cx, bufs),
            #[cfg(feature = "py-dynamic-openssl")]
            TlsStreamInner::PyDynamicOpenSsl(s) => Pin::new(s).poll_write_vectored(cx, bufs),
            #[cfg(not(any(
                feature = "native-tls",
                feature = "rustls",
                feature = "py-dynamic-openssl",
            )))]
            TlsStreamInner::None(f, ..) => match *f {},
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut self.get_mut().0 {
            #[cfg(feature = "native-tls")]
            TlsStreamInner::NativeTls(s) => Pin::new(s).poll_flush(cx),
            #[cfg(feature = "rustls")]
            TlsStreamInner::Rustls(s) => Pin::new(s).poll_flush(cx),
            #[cfg(feature = "py-dynamic-openssl")]
            TlsStreamInner::PyDynamicOpenSsl(s) => Pin::new(s).poll_flush(cx),
            #[cfg(not(any(
                feature = "native-tls",
                feature = "rustls",
                feature = "py-dynamic-openssl",
            )))]
            TlsStreamInner::None(f, ..) => match *f {},
        }
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut self.get_mut().0 {
            #[cfg(feature = "native-tls")]
            TlsStreamInner::NativeTls(s) => Pin::new(s).poll_close(cx),
            #[cfg(feature = "rustls")]
            TlsStreamInner::Rustls(s) => Pin::new(s).poll_close(cx),
            #[cfg(feature = "py-dynamic-openssl")]
            TlsStreamInner::PyDynamicOpenSsl(s) => Pin::new(s).poll_close(cx),
            #[cfg(not(any(
                feature = "native-tls",
                feature = "rustls",
                feature = "py-dynamic-openssl",
            )))]
            TlsStreamInner::None(f, ..) => match *f {},
        }
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin + 'static> AsyncWrite for TlsStream<S>
where
    for<'a> &'a S: AsyncRead + AsyncWrite,
{
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        let slice = buf.as_init();
        let res = futures_util::AsyncWriteExt::write(self, slice).await;
        BufResult(res, buf)
    }

    async fn write_vectored<T: IoVectoredBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        let slices = buf.iter_slice().map(io::IoSlice::new).collect::<Vec<_>>();
        let res = futures_util::AsyncWriteExt::write_vectored(self, &slices).await;
        BufResult(res, buf)
    }

    async fn flush(&mut self) -> io::Result<()> {
        futures_util::AsyncWriteExt::flush(self).await
    }

    async fn shutdown(&mut self) -> io::Result<()> {
        futures_util::AsyncWriteExt::close(self).await
    }
}
