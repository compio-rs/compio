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
    #[cfg(feature = "py-dynamic-openssl")]
    PyDynamicOpenSsl(compio_py_dynamic_openssl::ssl::SslStream<SyncStream<S>>),
    #[cfg(not(any(
        feature = "native-tls",
        feature = "rustls",
        feature = "py-dynamic-openssl",
    )))]
    None(std::convert::Infallible, std::marker::PhantomData<S>),
}

impl<S> TlsStreamInner<S> {
    pub fn negotiated_alpn(&self) -> Option<Cow<'_, [u8]>> {
        match self {
            #[cfg(feature = "native-tls")]
            Self::NativeTls(s) => s.negotiated_alpn().ok().flatten().map(Cow::from),
            #[cfg(feature = "rustls")]
            Self::Rustls(s) => s.get_ref().1.alpn_protocol().map(Cow::from),
            #[cfg(feature = "py-dynamic-openssl")]
            Self::PyDynamicOpenSsl(s) => py::negotiated_alpn(s),
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

#[cfg(feature = "native-tls")]
#[inline]
async fn drive<S, F, T>(s: &mut native_tls::TlsStream<SyncStream<S>>, mut f: F) -> io::Result<T>
where
    S: AsyncRead + AsyncWrite,
    F: FnMut(&mut native_tls::TlsStream<SyncStream<S>>) -> io::Result<T>,
{
    loop {
        match f(s) {
            Ok(res) => {
                let s = s.get_mut();
                if s.has_pending_write() {
                    s.flush_write_buf().await?;
                }
                break Ok(res);
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                let s = s.get_mut();
                if s.has_pending_write() {
                    s.flush_write_buf().await?;
                } else {
                    s.fill_read_buf().await?;
                }
            }
            Err(e) => break Err(e),
        }
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
            TlsStreamInner::NativeTls(s) => match drive(s, |s| io::Read::read(s, slice)).await {
                Ok(res) => {
                    unsafe { buf.advance_to(res) };
                    BufResult(Ok(res), buf)
                }
                res => BufResult(res, buf),
            },
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
            TlsStreamInner::PyDynamicOpenSsl(s) => match py::read(s, slice).await {
                Ok(res) => {
                    unsafe { buf.advance_to(res) };
                    BufResult(Ok(res), buf)
                }
                Err(e) => BufResult(Err(e), buf),
            },
            #[cfg(not(any(
                feature = "native-tls",
                feature = "rustls",
                feature = "py-dynamic-openssl",
            )))]
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
        let slice = buf.as_init();
        match &mut self.0 {
            #[cfg(feature = "native-tls")]
            TlsStreamInner::NativeTls(s) => {
                let res = drive(s, |s| io::Write::write(s, slice)).await;
                BufResult(res, buf)
            }
            #[cfg(feature = "rustls")]
            TlsStreamInner::Rustls(s) => {
                let res = futures_util::AsyncWriteExt::write(s, slice).await;
                BufResult(res, buf)
            }
            #[cfg(feature = "py-dynamic-openssl")]
            TlsStreamInner::PyDynamicOpenSsl(s) => {
                let res = py::write(s, slice).await;
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
            TlsStreamInner::NativeTls(s) => flush_impl(s).await,
            #[cfg(feature = "rustls")]
            TlsStreamInner::Rustls(s) => futures_util::AsyncWriteExt::flush(s).await,
            #[cfg(feature = "py-dynamic-openssl")]
            TlsStreamInner::PyDynamicOpenSsl(s) => s.get_mut().flush_write_buf().await.map(|_| ()),
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
                drive(s, |s| s.shutdown()).await?;
                s.get_mut().get_mut().shutdown().await
            }
            #[cfg(feature = "rustls")]
            TlsStreamInner::Rustls(s) => futures_util::AsyncWriteExt::close(s).await,
            #[cfg(feature = "py-dynamic-openssl")]
            TlsStreamInner::PyDynamicOpenSsl(s) => py::shutdown(s).await,
            #[cfg(not(any(
                feature = "native-tls",
                feature = "rustls",
                feature = "py-dynamic-openssl",
            )))]
            TlsStreamInner::None(f, ..) => match *f {},
        }
    }
}

#[cfg(feature = "py-dynamic-openssl")]
#[doc(hidden)]
mod py {
    use std::{borrow::Cow, io};

    use compio_io::{AsyncRead, AsyncWrite, compat::SyncStream};
    use compio_py_dynamic_openssl::ssl::{Error, ErrorCode, ShutdownResult, SslStream};

    use super::{TlsStream, TlsStreamInner};

    impl<S> From<SslStream<SyncStream<S>>> for TlsStream<S> {
        fn from(value: SslStream<SyncStream<S>>) -> Self {
            Self(TlsStreamInner::PyDynamicOpenSsl(value))
        }
    }

    enum DriveResult<T> {
        WantRead,
        WantWrite,
        Ready(io::Result<T>),
    }

    impl<T> From<Error> for DriveResult<T> {
        fn from(e: Error) -> Self {
            match e.code() {
                ErrorCode::WANT_READ => DriveResult::WantRead,
                ErrorCode::WANT_WRITE => DriveResult::WantWrite,
                _ => DriveResult::Ready(Err(e.into_io_error().unwrap_or_else(io::Error::other))),
            }
        }
    }

    impl<T> From<Result<T, Error>> for DriveResult<T> {
        fn from(res: Result<T, Error>) -> Self {
            match res {
                Ok(t) => DriveResult::Ready(Ok(t)),
                Err(e) => e.into(),
            }
        }
    }

    #[inline]
    async fn drive<S, F, T>(s: &mut SslStream<SyncStream<S>>, mut f: F) -> io::Result<T>
    where
        S: AsyncRead + AsyncWrite,
        F: FnMut(&mut SslStream<SyncStream<S>>) -> DriveResult<T>,
    {
        loop {
            let res = f(s);
            let s = s.get_mut();
            if s.has_pending_write() {
                s.flush_write_buf().await?;
            }
            match res {
                DriveResult::Ready(res) => break res,
                DriveResult::WantRead => _ = s.fill_read_buf().await?,
                DriveResult::WantWrite => {}
            }
        }
    }
    pub(crate) fn negotiated_alpn<S>(s: &SslStream<SyncStream<S>>) -> Option<Cow<'_, [u8]>> {
        s.ssl()
            .selected_alpn_protocol()
            .map(|alpn| alpn.to_vec())
            .map(Cow::from)
    }

    pub(crate) async fn read<S>(
        s: &mut SslStream<SyncStream<S>>,
        slice: &mut [u8],
    ) -> io::Result<usize>
    where
        S: AsyncRead + AsyncWrite,
    {
        drive(s, |s| match s.ssl_read(slice) {
            Ok(n) => DriveResult::Ready(Ok(n)),
            Err(e) => match e.code() {
                ErrorCode::ZERO_RETURN => DriveResult::Ready(Ok(0)),
                ErrorCode::SYSCALL if e.io_error().is_none() => DriveResult::Ready(Ok(0)),
                _ => e.into(),
            },
        })
        .await
    }

    pub(crate) async fn write<S>(
        s: &mut SslStream<SyncStream<S>>,
        slice: &[u8],
    ) -> io::Result<usize>
    where
        S: AsyncRead + AsyncWrite,
    {
        drive(s, |s| s.ssl_write(slice).into()).await
    }

    pub(crate) async fn shutdown<S>(s: &mut SslStream<SyncStream<S>>) -> io::Result<()>
    where
        S: AsyncRead + AsyncWrite,
    {
        let res = drive(s, |s| match s.shutdown() {
            Ok(res) => DriveResult::Ready(Ok(res)),
            Err(e) => {
                if e.code() == ErrorCode::ZERO_RETURN {
                    DriveResult::Ready(Ok(ShutdownResult::Received))
                } else {
                    e.into()
                }
            }
        })
        .await?;
        if let Err(e) = s.get_mut().get_mut().shutdown().await
            && e.kind() != io::ErrorKind::NotConnected
        {
            return Err(e);
        }
        match res {
            // If close_notify has been sent but the peer has not responded with
            // close_notify, we let the caller know by returning Err(WouldBlock).
            // This behavior is different from the others as a Python-only hack.
            ShutdownResult::Sent => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "close_notify sent",
            )),
            ShutdownResult::Received => Ok(()),
        }
    }
}
