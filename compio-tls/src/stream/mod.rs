use std::{io, mem::MaybeUninit};

use compio_buf::{BufResult, IoBuf, IoBufMut};
use compio_io::{compat::SyncStream, AsyncRead, AsyncWrite};

#[cfg(feature = "rustls")]
mod rtls;

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
enum TlsStreamInner<S> {
    #[cfg(feature = "native-tls")]
    NativeTls(native_tls::TlsStream<SyncStream<S>>),
    #[cfg(feature = "rustls")]
    Rustls(rtls::TlsStream<SyncStream<S>>),
}

impl<S> TlsStreamInner<S> {
    fn get_mut(&mut self) -> &mut SyncStream<S> {
        match self {
            #[cfg(feature = "native-tls")]
            Self::NativeTls(s) => s.get_mut(),
            #[cfg(feature = "rustls")]
            Self::Rustls(s) => s.get_mut(),
        }
    }
}

impl<S> io::Read for TlsStreamInner<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            #[cfg(feature = "native-tls")]
            Self::NativeTls(s) => io::Read::read(s, buf),
            #[cfg(feature = "rustls")]
            Self::Rustls(s) => io::Read::read(s, buf),
        }
    }

    #[cfg(feature = "read_buf")]
    fn read_buf(&mut self, buf: io::BorrowedCursor<'_>) -> io::Result<()> {
        match self {
            #[cfg(feature = "native-tls")]
            Self::NativeTls(s) => io::Read::read_buf(s, buf),
            #[cfg(feature = "rustls")]
            Self::Rustls(s) => io::Read::read_buf(s, buf),
        }
    }
}

impl<S> io::Write for TlsStreamInner<S> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            #[cfg(feature = "native-tls")]
            Self::NativeTls(s) => io::Write::write(s, buf),
            #[cfg(feature = "rustls")]
            Self::Rustls(s) => io::Write::write(s, buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            #[cfg(feature = "native-tls")]
            Self::NativeTls(s) => io::Write::flush(s),
            #[cfg(feature = "rustls")]
            Self::Rustls(s) => io::Write::flush(s),
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
    #[cfg(feature = "rustls")]
    pub(crate) fn new_rustls_client(s: SyncStream<S>, conn: rustls::ClientConnection) -> Self {
        Self(TlsStreamInner::Rustls(rtls::TlsStream::new_client(s, conn)))
    }

    #[cfg(feature = "rustls")]
    pub(crate) fn new_rustls_server(s: SyncStream<S>, conn: rustls::ServerConnection) -> Self {
        Self(TlsStreamInner::Rustls(rtls::TlsStream::new_server(s, conn)))
    }
}

#[cfg(feature = "native-tls")]
#[doc(hidden)]
impl<S> From<native_tls::TlsStream<SyncStream<S>>> for TlsStream<S> {
    fn from(value: native_tls::TlsStream<SyncStream<S>>) -> Self {
        Self(TlsStreamInner::NativeTls(value))
    }
}

#[cfg(not(feature = "read_buf"))]
#[inline]
fn read_buf<B: IoBufMut>(reader: &mut impl io::Read, buf: &mut B) -> io::Result<usize> {
    let slice: &mut [MaybeUninit<u8>] = buf.as_mut_slice();
    slice.fill(MaybeUninit::new(0));
    let slice = unsafe { std::slice::from_raw_parts_mut(slice.as_mut_ptr().cast(), slice.len()) };
    reader.read(slice)
}

#[cfg(feature = "read_buf")]
#[inline]
fn read_buf<B: IoBufMut>(reader: &mut impl io::Read, buf: &mut B) -> io::Result<usize> {
    let slice: &mut [MaybeUninit<u8>] = buf.as_mut_slice();
    let mut borrowed_buf = io::BorrowedBuf::from(slice);
    let mut cursor = borrowed_buf.unfilled();
    reader.read_buf(cursor.reborrow())?;
    Ok(cursor.written())
}

impl<S: AsyncRead> AsyncRead for TlsStream<S> {
    async fn read<B: IoBufMut>(&mut self, mut buf: B) -> BufResult<usize, B> {
        loop {
            let res = read_buf(&mut self.0, &mut buf);
            match res {
                Ok(res) => {
                    unsafe { buf.set_buf_init(res) };
                    return BufResult(Ok(res), buf);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    match self.0.get_mut().fill_read_buf().await {
                        Ok(_) => continue,
                        Err(e) => return BufResult(Err(e), buf),
                    }
                }
                _ => return BufResult(res, buf),
            }
        }
    }
}

impl<S: AsyncWrite> AsyncWrite for TlsStream<S> {
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        let slice = buf.as_slice();
        loop {
            let res = io::Write::write(&mut self.0, slice);
            match res {
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => match self.flush().await {
                    Ok(_) => continue,
                    Err(e) => return BufResult(Err(e), buf),
                },
                _ => return BufResult(res, buf),
            }
        }
    }

    async fn flush(&mut self) -> io::Result<()> {
        loop {
            match io::Write::flush(&mut self.0) {
                Ok(()) => break,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    self.0.get_mut().flush_write_buf().await?;
                }
                Err(e) => return Err(e),
            }
        }
        self.0.get_mut().flush_write_buf().await?;
        Ok(())
    }

    async fn shutdown(&mut self) -> io::Result<()> {
        self.0.get_mut().get_mut().shutdown().await
    }
}
