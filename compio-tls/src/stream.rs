use std::{borrow::Cow, io, mem::MaybeUninit, pin::Pin};

use compio_buf::{BufResult, IoBuf, IoBufMut};
use compio_io::{AsyncRead, AsyncWrite, compat::AsyncStream};

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
enum TlsStreamInner<S> {
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
pub struct TlsStream<S>(TlsStreamInner<S>);

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
impl<S> From<futures_rustls::client::TlsStream<Pin<Box<AsyncStream<S>>>>> for TlsStream<S> {
    fn from(value: futures_rustls::client::TlsStream<Pin<Box<AsyncStream<S>>>>) -> Self {
        Self(TlsStreamInner::Rustls(futures_rustls::TlsStream::Client(
            value,
        )))
    }
}

#[cfg(feature = "rustls")]
#[doc(hidden)]
impl<S> From<futures_rustls::server::TlsStream<Pin<Box<AsyncStream<S>>>>> for TlsStream<S> {
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

impl<S: AsyncRead + AsyncWrite + Unpin + 'static> AsyncRead for TlsStream<S>
where
    for<'a> &'a S: AsyncRead + AsyncWrite,
{
    async fn read<B: IoBufMut>(&mut self, mut buf: B) -> BufResult<usize, B> {
        let slice = buf.as_uninit();
        slice.fill(MaybeUninit::new(0));
        // SAFETY: The memory has been initialized
        let slice =
            unsafe { std::slice::from_raw_parts_mut::<u8>(slice.as_mut_ptr().cast(), slice.len()) };
        match &mut self.0 {
            #[cfg(feature = "native-tls")]
            TlsStreamInner::NativeTls(s) => {
                match futures_util::AsyncReadExt::read(s, slice).await {
                    Ok(res) => {
                        unsafe { buf.advance_to(res) };
                        BufResult(Ok(res), buf)
                    }
                    res => BufResult(res, buf),
                }
            }
            #[cfg(feature = "rustls")]
            TlsStreamInner::Rustls(s) => {
                let res = futures_util::AsyncReadExt::read(s, slice).await;
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
            #[cfg(feature = "py-dynamic-openssl")]
            TlsStreamInner::PyDynamicOpenSsl(s) => {
                match futures_util::AsyncReadExt::read(s, slice).await {
                    Ok(res) => {
                        unsafe { buf.advance_to(res) };
                        BufResult(Ok(res), buf)
                    }
                    Err(e) => BufResult(Err(e), buf),
                }
            }
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
        match &mut self.0 {
            #[cfg(feature = "native-tls")]
            TlsStreamInner::NativeTls(s) => {
                let res = futures_util::AsyncWriteExt::write(s, slice).await;
                BufResult(res, buf)
            }
            #[cfg(feature = "rustls")]
            TlsStreamInner::Rustls(s) => {
                let res = futures_util::AsyncWriteExt::write(s, slice).await;
                BufResult(res, buf)
            }
            #[cfg(feature = "py-dynamic-openssl")]
            TlsStreamInner::PyDynamicOpenSsl(s) => {
                let res = futures_util::AsyncWriteExt::write(s, slice).await;
                BufResult(res, buf)
            }
            #[cfg(not(any(
                feature = "native-tls",
                feature = "rustls",
                feature = "py-dynamic-openssl",
            )))]
            TlsStreamInner::None(f, ..) => match *f {},
        }
    }

    async fn flush(&mut self) -> io::Result<()> {
        match &mut self.0 {
            #[cfg(feature = "native-tls")]
            TlsStreamInner::NativeTls(s) => futures_util::AsyncWriteExt::flush(s).await,
            #[cfg(feature = "rustls")]
            TlsStreamInner::Rustls(s) => futures_util::AsyncWriteExt::flush(s).await,
            #[cfg(feature = "py-dynamic-openssl")]
            TlsStreamInner::PyDynamicOpenSsl(s) => futures_util::AsyncWriteExt::flush(s).await,
            #[cfg(not(any(
                feature = "native-tls",
                feature = "rustls",
                feature = "py-dynamic-openssl",
            )))]
            TlsStreamInner::None(f, ..) => match *f {},
        }
    }

    async fn shutdown(&mut self) -> io::Result<()> {
        self.flush().await?;
        match &mut self.0 {
            #[cfg(feature = "native-tls")]
            TlsStreamInner::NativeTls(s) => {
                // Send close_notify alert, then shutdown the underlying stream.
                // Note, this implementation is platform-specific relying on how
                // native-tls handles shutdown. In general, it's consistent on
                // first call (sending close_notify); but it may or may not block
                // and wait for the peer to respond with close_notify on any
                // subsequent calls. Here we just let such behavior propagate,
                // and suggest the users to call shutdown() at most once.
                futures_util::AsyncWriteExt::close(s).await?;
                futures_util::AsyncWriteExt::close(s.get_mut().get_mut().get_mut()).await
            }
            #[cfg(feature = "rustls")]
            TlsStreamInner::Rustls(s) => futures_util::AsyncWriteExt::close(s).await,
            #[cfg(feature = "py-dynamic-openssl")]
            TlsStreamInner::PyDynamicOpenSsl(s) => futures_util::AsyncWriteExt::close(s).await,
            #[cfg(not(any(
                feature = "native-tls",
                feature = "rustls",
                feature = "py-dynamic-openssl",
            )))]
            TlsStreamInner::None(f, ..) => match *f {},
        }
    }
}
