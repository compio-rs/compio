use std::io;

use crate::{IntoInner, IoBuf, IoBufMut};

/// Adapts an [`IoBuf`] to implement the [`std::io::Read`] trait.
///
/// This can be constructed with [`IoBuf::into_reader`]
pub struct Reader<B>(pub(crate) B);

impl<B: IoBuf> io::Read for Reader<B> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.as_init().read(buf)
    }
}

impl<B> Reader<B> {
    /// Returns a reference to the inner buffer.
    pub fn as_inner(&self) -> &B {
        &self.0
    }

    /// Returns a mutable reference to the inner buffer.
    pub fn as_inner_mut(&mut self) -> &mut B {
        &mut self.0
    }
}

impl<B> IntoInner for Reader<B> {
    type Inner = B;

    fn into_inner(self) -> Self::Inner {
        self.0
    }
}

/// Adapts a reference to [`IoBuf`] to [`std::io::Read`] trait.
///
/// This can be constructed with [`IoBuf::as_reader`].
pub struct ReaderRef<'a, B: ?Sized>(pub(crate) &'a B);

impl<B: IoBuf> io::Read for ReaderRef<'_, B> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.as_init().read(buf)
    }
}

/// Adapts an [`IoBufMut`] to implement the [`std::io::Write`] trait.
///
/// This can be constructed with [`IoBufMut::into_writer`]
pub struct Writer<B>(pub(crate) B);

impl<B> Writer<B> {
    /// Returns a reference to the inner buffer.
    pub fn as_inner(&self) -> &B {
        &self.0
    }

    /// Returns a mutable reference to the inner buffer.
    pub fn as_inner_mut(&mut self) -> &mut B {
        &mut self.0
    }
}

impl<B: IoBufMut> io::Write for Writer<B> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.0.extend_from_slice(buf) {
            Ok(_) => Ok(buf.len()),
            Err(e) => Err(io::Error::other(e)),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<B> IntoInner for Writer<B> {
    type Inner = B;

    fn into_inner(self) -> Self::Inner {
        self.0
    }
}

/// Adapts a mutable reference to [`IoBufMut`] to [`std::io::Write`] trait.
///
/// This can be constructed with [`IoBufMut::as_writer`].
pub struct WriterRef<'a, B: ?Sized>(pub(crate) &'a mut B);

impl<B: ?Sized> WriterRef<'_, B> {
    /// Returns a reference to the inner buffer.
    pub fn as_inner(&self) -> &B {
        self.0
    }

    /// Returns a mutable reference to the inner buffer.
    pub fn as_inner_mut(&mut self) -> &mut B {
        self.0
    }
}

impl<B: IoBufMut + ?Sized> io::Write for WriterRef<'_, B> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.0.extend_from_slice(buf) {
            Ok(_) => Ok(buf.len()),
            Err(e) => Err(io::Error::other(e)),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
