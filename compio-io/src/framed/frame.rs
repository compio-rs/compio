//! Traits and implementations for frame extraction and enclosing

use compio_buf::{
    IoBufMut,
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
    pub fn payload<'a>(&self, buf: &'a [u8]) -> &'a [u8] {
        &buf[self.prefix..self.prefix + self.payload]
    }

    /// Shifts bytes after the frame to the beginning of `buf`
    pub fn consume(&self, buf: &mut [u8]) {
        buf.copy_within(self.len().., 0);
    }
}

/// Enclosing and extracting frames in a buffer.
pub trait Framer {
    /// Enclose a frame in the given buffer.
    ///
    /// All initialized bytes in `buf` (`buf[0..buf.len()]`) are valid and
    /// required to be enclosed. All modifications should happen in-place; one
    /// can use [`slice::copy_within`] or a temporary buffer if prepending data
    /// is necessary.
    ///
    /// [`slice::copy_within`]: https://doc.rust-lang.org/std/primitive.slice.html#method.copy_within
    fn enclose(&mut self, buf: &mut Vec<u8>);

    /// Extract a frame from the given buffer.
    ///
    /// # Returns
    /// - `Ok(Some(frame))` if a complete frame is found.
    /// - `Ok(None)` if no complete frame is found.
    /// - `Err(io::Error)` if an error occurs during extraction.
    fn extract(&mut self, buf: &[u8]) -> Option<Frame>;
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

impl Framer for LengthDelimited {
    fn enclose(&mut self, buf: &mut Vec<u8>) {
        let len = buf.len();

        buf.reserve(self.length_field_len);
        IoBufMut::as_mut_slice(buf).copy_within(0..len, self.length_field_len); // Shift existing data
        unsafe { buf.set_len(len + self.length_field_len) };

        // Write the length at the beginning
        if self.length_field_is_big_endian {
            (&mut buf[0..self.length_field_len]).put_uint(len as _, self.length_field_len);
        } else {
            (&mut buf[0..self.length_field_len]).put_uint_le(len as _, self.length_field_len);
        }
    }

    fn extract(&mut self, mut buf: &[u8]) -> Option<Frame> {
        if buf.len() < self.length_field_len {
            return None;
        }

        let len = if self.length_field_is_big_endian {
            buf.get_uint(self.length_field_len)
        } else {
            buf.get_uint_le(self.length_field_len)
        } as usize;

        if buf.len() < len {
            return None;
        }

        Some(Frame::new(self.length_field_len, len, 0))
    }
}

/// A generic delimiter that uses a single character.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CharDelimited<const C: char> {}

impl<const C: char> CharDelimited<C> {
    /// Creates a new `CharDelimited`
    pub fn new() -> Self {
        Self {}
    }
}

impl<const C: char> Framer for CharDelimited<C> {
    fn enclose(&mut self, buf: &mut Vec<u8>) {
        buf.push(C as u8);
    }

    fn extract(&mut self, buf: &[u8]) -> Option<Frame> {
        if buf.is_empty() {
            return None;
        }

        buf.iter()
            .position(|&b| b == C as u8)
            .map(|pos| Frame::new(0, pos, 1))
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

impl Framer for AnyDelimited<'_> {
    fn extract(&mut self, buf: &[u8]) -> Option<Frame> {
        if buf.is_empty() {
            return None;
        }

        // Search for the first occurrence of any byte in `self.bytes`
        // TODO(George-Miao): Optimize if performance is a concern
        if let Some(pos) = buf
            .windows(self.bytes.len())
            .position(|window| window == self.bytes)
        {
            Some(Frame::new(0, pos, self.bytes.len()))
        } else {
            None
        }
    }

    fn enclose(&mut self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(self.bytes);
    }
}

/// Delimiter that uses newline characters (`\n`) as delimiters.
pub type LineDelimited = CharDelimited<'\n'>;
