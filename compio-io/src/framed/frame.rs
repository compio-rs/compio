//! Traits and implementations for frame extraction and enclosing

use compio_buf::IoBufMut;

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

/// Embedding and extracting bytes into buf in a framed fashion.
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
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LengthDelimited {}

impl LengthDelimited {
    /// Creates a new `LengthDelimited` framer.
    pub fn new() -> Self {
        Self {}
    }
}

impl Framer for LengthDelimited {
    fn enclose(&mut self, buf: &mut Vec<u8>) {
        let len = buf.len();

        buf.reserve(8);
        IoBufMut::as_mut_slice(buf).copy_within(0..len, 8); // Shift existing data
        unsafe { buf.set_len(len + 8) };
        buf[0..8].copy_from_slice(&(len as u64).to_be_bytes()); // Write the length at the beginning
    }

    fn extract(&mut self, buf: &[u8]) -> Option<Frame> {
        if buf.len() < 8 {
            return None;
        }

        let len = u64::from_be_bytes(buf[0..8].try_into().unwrap()) as usize;

        if buf.len() < len + 8 {
            return None;
        }

        Some(Frame::new(8, len, 0))
    }
}

/// A generic delimiter that uses a single character.
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

impl<const C: char> Default for CharDelimited<C> {
    fn default() -> Self {
        Self {}
    }
}

/// A generic delimiter that uses any sequence of bytes as a delimiter.
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
