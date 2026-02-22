use std::{borrow::Cow, io};

use compio_io::{AsyncRead, AsyncWrite, compat::SyncStream};
use compio_py_dynamic_openssl::ssl::{Error, ErrorCode, ShutdownResult, SslStream};

use crate::TlsStream;

pub(crate) async fn handshake<S: AsyncRead + AsyncWrite>(
    mut res: Result<
        compio_py_dynamic_openssl::ssl::SslStream<SyncStream<S>>,
        compio_py_dynamic_openssl::ssl::HandshakeError<SyncStream<S>>,
    >,
) -> io::Result<TlsStream<S>> {
    use compio_py_dynamic_openssl::ssl::HandshakeError;

    loop {
        match res {
            Ok(mut s) => {
                let inner = s.get_mut();
                if inner.has_pending_write() {
                    inner.flush_write_buf().await?;
                }
                return Ok(TlsStream::from(s));
            }
            Err(e) => match e {
                HandshakeError::SetupFailure(e) => return Err(io::Error::other(e)),
                HandshakeError::Failure(mid_stream) => {
                    return Err(io::Error::other(mid_stream.into_error()));
                }
                HandshakeError::WouldBlock(mut mid_stream) => {
                    let s = mid_stream.get_mut();
                    if s.has_pending_write() {
                        s.flush_write_buf().await?;
                    } else {
                        s.fill_read_buf().await?;
                    }
                    res = mid_stream.handshake();
                }
            },
        }
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

pub(crate) async fn read<S>(s: &mut SslStream<SyncStream<S>>, slice: &mut [u8]) -> io::Result<usize>
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

pub(crate) async fn write<S>(s: &mut SslStream<SyncStream<S>>, slice: &[u8]) -> io::Result<usize>
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
