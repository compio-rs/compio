//! Traits and implementations for frame extraction and enclosing

use std::io;

use compio_buf::{
    IoBuf, IoBufMut, Slice,
    bytes::{Buf, BufMut},
};

/// An extracted frame
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    /// Offset where the frame payload begins
    prefix: usize,

    /// Length of the frame payload
    payload: usize,

    /// Suffix length of the frame
    suffix: usize,
}

impl Frame {
    /// Create a new [`Frame`] with the specified prefix, payload, and suffix
    /// lengths.
    pub fn new(prefix: usize, payload: usize, suffix: usize) -> Self {
        Self {
            prefix,
            payload,
            suffix,
        }
    }

    /// Length of the entire frame
    pub fn len(&self) -> usize {
        self.prefix + self.payload + self.suffix
    }

    /// If the frame is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Slice payload out of the buffer
    pub fn slice<B: IoBuf>(&self, buf: B) -> Slice<B> {
        buf.slice(self.prefix..self.prefix + self.payload)
    }
}

/// Enclosing and extracting frames in a buffer.
pub trait Framer<B: IoBufMut> {
    /// Enclose a frame in the given buffer.
    ///
    /// All initialized bytes in `buf` (`buf[0..buf.buf_len()]`) are valid and
    /// required to be enclosed. All modifications should happen in-place; one
    /// can use [`IoBufMut::reserve`], [`IoBufMut::copy_within`] or a temporary
    /// buffer if prepending data is necessary.
    ///
    /// [`slice::copy_within`]: https://doc.rust-lang.org/std/primitive.slice.html#method.copy_within
    fn enclose(&mut self, buf: &mut B);

    /// Extract a frame from the given buffer.
    ///
    /// # Returns
    /// - `Ok(Some(frame))` if a complete frame is found.
    /// - `Ok(None)` if no complete frame is found.
    /// - `Err(io::Error)` if an error occurs during extraction.
    fn extract(&mut self, buf: &Slice<B>) -> io::Result<Option<Frame>>;
}

/// A simple extractor that frames data by its length.
///
/// It uses 8 bytes to represent the length of the data at the beginning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LengthDelimited {
    length_field_len: usize,
    length_field_is_big_endian: bool,
}

impl Default for LengthDelimited {
    fn default() -> Self {
        Self {
            length_field_len: 4,
            length_field_is_big_endian: true,
        }
    }
}

impl LengthDelimited {
    /// Creates a new `LengthDelimited` framer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the length of the length field in bytes.
    pub fn length_field_len(&self) -> usize {
        self.length_field_len
    }

    /// Sets the length of the length field in bytes.
    pub fn set_length_field_len(mut self, len_field_len: usize) -> Self {
        self.length_field_len = len_field_len;
        self
    }

    /// Returns whether the length field is big-endian.
    pub fn length_field_is_big_endian(&self) -> bool {
        self.length_field_is_big_endian
    }

    /// Sets whether the length field is big-endian.
    pub fn set_length_field_is_big_endian(mut self, big_endian: bool) -> Self {
        self.length_field_is_big_endian = big_endian;
        self
    }
}

impl<B: IoBufMut> Framer<B> for LengthDelimited {
    fn enclose(&mut self, buf: &mut B) {
        let len = (*buf).buf_len();

        buf.reserve(self.length_field_len).expect("Reserve failed");
        buf.copy_within(0..len, self.length_field_len); // Shift existing data
        unsafe { buf.advance_to(len + self.length_field_len) };

        let slice = buf.as_mut_slice();

        // Write the length at the beginning
        if self.length_field_is_big_endian {
            (&mut slice[0..self.length_field_len]).put_uint(len as _, self.length_field_len);
        } else {
            (&mut slice[0..self.length_field_len]).put_uint_le(len as _, self.length_field_len);
        }
    }

    fn extract(&mut self, buf: &Slice<B>) -> io::Result<Option<Frame>> {
        if buf.len() < self.length_field_len {
            return Ok(None);
        }

        let mut buf = buf.as_init();

        let len = if self.length_field_is_big_endian {
            buf.get_uint(self.length_field_len)
        } else {
            buf.get_uint_le(self.length_field_len)
        } as usize;

        if buf.len() < len {
            return Ok(None);
        }

        Ok(Some(Frame::new(self.length_field_len, len, 0)))
    }
}

/// A generic delimiter that uses a single character encoded as UTF-8.
///
/// If you need to use a multi-byte delimiter or other encodings, consider using
/// [`AnyDelimited`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CharDelimited<const C: char> {
    char_buf: [u8; 4],
}

impl<const C: char> CharDelimited<C> {
    /// Creates a new `CharDelimited`
    pub fn new() -> Self {
        Self { char_buf: [0; 4] }
    }

    fn as_any_delimited(&mut self) -> AnyDelimited<'_> {
        let bytes = C.encode_utf8(&mut self.char_buf).as_bytes();

        AnyDelimited::new(bytes)
    }
}

impl<B: IoBufMut, const C: char> Framer<B> for CharDelimited<C> {
    fn enclose(&mut self, buf: &mut B) {
        self.as_any_delimited().enclose(buf);
    }

    fn extract(&mut self, buf: &Slice<B>) -> io::Result<Option<Frame>> {
        self.as_any_delimited().extract(buf)
    }
}

/// A generic delimiter that uses any sequence of bytes as a delimiter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AnyDelimited<'a> {
    bytes: &'a [u8],
}

impl<'a> AnyDelimited<'a> {
    /// Creates a new `AnyDelimited` with the specified delimiter bytes.
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }
}

impl<B: IoBufMut> Framer<B> for AnyDelimited<'_> {
    fn extract(&mut self, buf: &Slice<B>) -> io::Result<Option<Frame>> {
        if buf.is_empty() {
            return Ok(None);
        }

        // Search for the first occurrence of any byte in `self.bytes`
        // TODO(George-Miao): Optimize with memchr if performance is a concern
        if let Some(pos) = buf
            .windows(self.bytes.len())
            .position(|window| window == self.bytes)
        {
            Ok(Some(Frame::new(0, pos, self.bytes.len())))
        } else {
            Ok(None)
        }
    }

    fn enclose(&mut self, buf: &mut B) {
        buf.extend_from_slice(self.bytes)
            .expect("Failed to append delimiter");
    }
}

/// Delimiter that uses newline characters (`\n`) as delimiters.
pub type LineDelimited = CharDelimited<'\n'>;

#[cfg(test)]
mod tests {
    use compio_buf::{IntoInner, IoBufMut};

    use super::*;

    #[test]
    fn test_length_delimited() {
        let mut framer = LengthDelimited::new();

        let mut buf = Vec::from(b"hello");
        framer.enclose(&mut buf);
        assert_eq!(&buf.as_slice()[..9], b"\x00\x00\x00\x05hello");

        let buf = buf.slice(..);
        let frame = framer.extract(&buf).unwrap().unwrap();
        let buf = buf.into_inner();
        assert_eq!(frame, Frame::new(4, 5, 0));
        let payload = frame.slice(buf);
        assert_eq!(payload.as_init(), b"hello");
    }

    #[test]
    fn test_char_delimited() {
        let mut framer = CharDelimited::<'ℝ'>::new();

        let mut buf = Vec::new();
        IoBufMut::extend_from_slice(&mut buf, b"hello").unwrap();
        framer.enclose(&mut buf);
        assert_eq!(buf.as_slice(), "helloℝ".as_init());

        let buf = buf.slice(..);
        let frame = framer.extract(&buf).unwrap().unwrap();
        assert_eq!(frame, Frame::new(0, 5, 3));
        let payload = frame.slice(buf);
        assert_eq!(payload.as_init(), b"hello");
    }
}
