use std::{io, marker::PhantomData};

use crate::{IntoInner, IoBuf, IoBufMut, Slice};

/// Adapts an [`IoBuf`] to implement the [`std::io::Read`] trait.
///
/// This can be constructed with [`IoBuf::into_reader`]
pub struct Reader<B>(Slice<B>);

impl<B> Reader<B> {
    /// Creates a new [`Reader`] from the given buffer.
    pub fn new(buf: B) -> Self
    where
        B: IoBuf,
    {
        Self(buf.slice(..))
    }

    /// Returns a reference to the inner buffer.
    pub fn as_inner(&self) -> &B {
        self.0.as_inner()
    }

    /// Returns a mutable reference to the inner buffer.
    pub fn as_inner_mut(&mut self) -> &mut B {
        self.0.as_inner_mut()
    }

    /// Returns the number of bytes that have been read so far.
    pub fn progress(&self) -> usize {
        self.0.begin()
    }
}

impl<B: IoBuf> Reader<B> {
    /// Returns the remaining bytes to be read.
    pub fn as_remaining(&self) -> &[u8] {
        &self.0
    }

    /// Consumes the reader and returns the remaining bytes to be read.
    pub fn into_remaining(self) -> Slice<B> {
        self.0
    }
}

impl<B: IoBuf> io::Read for Reader<B> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.as_remaining().read(buf)?;
        self.0.set_begin(self.0.begin() + n);
        Ok(n)
    }
}

impl<B> IntoInner for Reader<B> {
    type Inner = B;

    fn into_inner(self) -> Self::Inner {
        self.0.into_inner()
    }
}

/// Adapts a reference to [`IoBuf`] to [`std::io::Read`] trait.
///
/// This can be constructed with [`IoBuf::as_reader`].
pub struct ReaderRef<'a, B: ?Sized>(&'a [u8], PhantomData<&'a B>);

impl<'a, B: ?Sized> ReaderRef<'a, B> {
    /// Creates a new [`ReaderRef`] from the given buffer reference.
    pub fn new(buf: &'a B) -> Self
    where
        B: IoBuf,
    {
        Self(buf.as_init(), PhantomData)
    }
}

impl<B: IoBuf> io::Read for ReaderRef<'_, B> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // implementation of Read for &[u8] will update the reference to point after
        // the read bytes, so we can just delegate to it.
        self.0.read(buf)
    }
}

/// Adapts an [`IoBufMut`] to implement the [`std::io::Write`] trait.
///
/// This can be constructed with [`IoBufMut::into_writer`]
pub struct Writer<B>(B);

impl<B> Writer<B> {
    /// Creates a new [`Writer`] from the given buffer.
    pub fn new(buf: B) -> Self {
        Self(buf)
    }

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
pub struct WriterRef<'a, B: ?Sized>(&'a mut B);

impl<'a, B: ?Sized> WriterRef<'a, B> {
    /// Creates a new [`WriterRef`] from the given mutable buffer reference.
    pub fn new(buf: &'a mut B) -> Self {
        Self(buf)
    }

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
        self.0.extend_from_slice(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

    use super::*;

    #[test]
    fn reader_tracks_progress_and_remaining() {
        let data = b"hello".to_vec();
        let mut reader = Reader::new(data.clone());

        let mut chunk = [0u8; 2];
        let n = reader.read(&mut chunk).unwrap();
        assert_eq!(n, 2);
        assert_eq!(&chunk, b"he");
        assert_eq!(reader.progress(), 2);
        assert_eq!(reader.as_remaining(), b"llo");

        let remaining = reader.into_remaining();
        assert_eq!(&*remaining, b"llo");
    }

    #[test]
    fn reader_ref_delegates_reads() {
        let data = b"readref".to_vec();
        let mut reader = ReaderRef::new(&data);

        let mut first = [0u8; 4];
        _ = reader.read(&mut first).unwrap();
        assert_eq!(&first, b"read");

        let mut second = [0u8; 3];
        _ = reader.read(&mut second).unwrap();
        assert_eq!(&second, b"ref");
    }

    #[test]
    fn writer_accumulates_bytes() {
        let mut writer = Writer::new(Vec::new());

        writer.write_all(b"foo").unwrap();
        writer.write_all(b"bar").unwrap();
        writer.flush().unwrap();

        assert_eq!(writer.as_inner().as_slice(), b"foobar");

        let inner = writer.into_inner();
        assert_eq!(inner, b"foobar".to_vec());
    }

    #[test]
    fn writer_ref_updates_underlying_buffer() {
        let mut buf = Vec::new();

        {
            let mut writer = WriterRef::new(&mut buf);
            writer.write_all(b"abc").unwrap();
            writer.write_all(b"123").unwrap();
            writer.flush().unwrap();
        }

        assert_eq!(buf, b"abc123");
    }
}
