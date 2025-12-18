use std::{borrow::Cow, io, mem::MaybeUninit};

use compio_buf::{BufResult, IoBuf, IoBufMut};
use compio_io::{
    AsyncRead, AsyncWrite,
    compat::{AsyncStream, SyncStream},
};

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
enum TlsStreamInner<S> {
    #[cfg(feature = "native-tls")]
    NativeTls(native_tls::TlsStream<SyncStream<S>>),
    #[cfg(feature = "rustls")]
    Rustls(futures_rustls::TlsStream<AsyncStream<S>>),
    #[cfg(not(any(feature = "native-tls", feature = "rustls")))]
    None(std::convert::Infallible, std::marker::PhantomData<S>),
}

impl<S> TlsStreamInner<S> {
    pub fn negotiated_alpn(&self) -> Option<Cow<'_, [u8]>> {
        match self {
            #[cfg(feature = "native-tls")]
            Self::NativeTls(s) => s.negotiated_alpn().ok().flatten().map(Cow::from),
            #[cfg(feature = "rustls")]
            Self::Rustls(s) => s.get_ref().1.alpn_protocol().map(Cow::from),
            #[cfg(not(any(feature = "native-tls", feature = "rustls")))]
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

impl<S> TlsStream<S> {
    /// Returns the negotiated ALPN protocol.
    pub fn negotiated_alpn(&self) -> Option<Cow<'_, [u8]>> {
        self.0.negotiated_alpn()
    }
}

#[cfg(feature = "native-tls")]
#[doc(hidden)]
impl<S> From<native_tls::TlsStream<SyncStream<S>>> for TlsStream<S> {
    fn from(value: native_tls::TlsStream<SyncStream<S>>) -> Self {
        Self(TlsStreamInner::NativeTls(value))
    }
}

#[cfg(feature = "rustls")]
#[doc(hidden)]
impl<S> From<futures_rustls::client::TlsStream<AsyncStream<S>>> for TlsStream<S> {
    fn from(value: futures_rustls::client::TlsStream<AsyncStream<S>>) -> Self {
        Self(TlsStreamInner::Rustls(futures_rustls::TlsStream::Client(
            value,
        )))
    }
}

#[cfg(feature = "rustls")]
#[doc(hidden)]
impl<S> From<futures_rustls::server::TlsStream<AsyncStream<S>>> for TlsStream<S> {
    fn from(value: futures_rustls::server::TlsStream<AsyncStream<S>>) -> Self {
        Self(TlsStreamInner::Rustls(futures_rustls::TlsStream::Server(
            value,
        )))
    }
}

impl<S: AsyncRead + AsyncWrite + 'static> AsyncRead for TlsStream<S> {
    async fn read<B: IoBufMut>(&mut self, mut buf: B) -> BufResult<usize, B> {
        let slice = buf.as_uninit();
        slice.fill(MaybeUninit::new(0));
        // SAFETY: The memory has been initialized
        let slice =
            unsafe { std::slice::from_raw_parts_mut::<u8>(slice.as_mut_ptr().cast(), slice.len()) };
        match &mut self.0 {
            #[cfg(feature = "native-tls")]
            TlsStreamInner::NativeTls(s) => loop {
                match io::Read::read(s, slice) {
                    Ok(res) => {
                        return BufResult(Ok(res), buf);
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        match s.get_mut().fill_read_buf().await {
                            Ok(_) => continue,
                            Err(e) => return BufResult(Err(e), buf),
                        }
                    }
                    res => return BufResult(res, buf),
                }
            },
            #[cfg(feature = "rustls")]
            TlsStreamInner::Rustls(s) => {
                let res = futures_util::AsyncReadExt::read(s, slice).await;
                let res = match res {
                    Ok(len) => Ok(len),
                    // TLS streams may return UnexpectedEof when the connection is closed.
                    // https://docs.rs/rustls/latest/rustls/manual/_03_howto/index.html#unexpected-eof
                    Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Ok(0),
                    _ => res,
                };
                BufResult(res, buf)
            }
            #[cfg(not(any(feature = "native-tls", feature = "rustls")))]
            TlsStreamInner::None(f, ..) => match *f {},
        }
    }
}

#[cfg(feature = "native-tls")]
async fn flush_impl(s: &mut native_tls::TlsStream<SyncStream<impl AsyncWrite>>) -> io::Result<()> {
    loop {
        match io::Write::flush(s) {
            Ok(()) => break,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                s.get_mut().flush_write_buf().await?;
            }
            Err(e) => return Err(e),
        }
    }
    s.get_mut().flush_write_buf().await?;
    Ok(())
}

impl<S: AsyncRead + AsyncWrite + 'static> AsyncWrite for TlsStream<S> {
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        let slice = buf.as_slice();
        match &mut self.0 {
            #[cfg(feature = "native-tls")]
            TlsStreamInner::NativeTls(s) => loop {
                let res = io::Write::write(s, slice);
                match res {
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => match flush_impl(s).await {
                        Ok(_) => continue,
                        Err(e) => return BufResult(Err(e), buf),
                    },
                    _ => return BufResult(res, buf),
                }
            },
            #[cfg(feature = "rustls")]
            TlsStreamInner::Rustls(s) => {
                let res = futures_util::AsyncWriteExt::write(s, slice).await;
                BufResult(res, buf)
            }
            #[cfg(not(any(feature = "native-tls", feature = "rustls")))]
            TlsStreamInner::None(f, ..) => match *f {},
        }
    }

    async fn flush(&mut self) -> io::Result<()> {
        match &mut self.0 {
            #[cfg(feature = "native-tls")]
            TlsStreamInner::NativeTls(s) => flush_impl(s).await,
            #[cfg(feature = "rustls")]
            TlsStreamInner::Rustls(s) => futures_util::AsyncWriteExt::flush(s).await,
            #[cfg(not(any(feature = "native-tls", feature = "rustls")))]
            TlsStreamInner::None(f, ..) => match *f {},
        }
    }

    async fn shutdown(&mut self) -> io::Result<()> {
        self.flush().await?;
        match &mut self.0 {
            #[cfg(feature = "native-tls")]
            TlsStreamInner::NativeTls(s) => s.get_mut().get_mut().shutdown().await,
            #[cfg(feature = "rustls")]
            TlsStreamInner::Rustls(s) => futures_util::AsyncWriteExt::close(s).await,
            #[cfg(not(any(feature = "native-tls", feature = "rustls")))]
            TlsStreamInner::None(f, ..) => match *f {},
        }
    }
}
