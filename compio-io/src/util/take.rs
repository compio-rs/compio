use compio_buf::{buf_try, BufResult, IntoInner, IoBufMut};

use crate::{AsyncBufRead, AsyncRead, IoResult};

/// Read up to a limit number of bytes from reader.
#[derive(Debug)]
pub struct Take<R> {
    reader: R,
    limit: u64,
}

impl<T> Take<T> {
    pub(crate) fn new(reader: T, limit: u64) -> Self {
        Self { reader, limit }
    }

    /// Returns the number of bytes that can be read before this instance will
    /// return EOF.
    ///
    /// # Note
    ///
    /// This instance may reach `EOF` after reading fewer bytes than indicated
    /// by this method if the underlying [`AsyncRead`] instance reaches EOF.
    pub fn limit(&self) -> u64 {
        self.limit
    }

    /// Sets the number of bytes that can be read before this instance will
    /// return EOF. This is the same as constructing a new `Take` instance, so
    /// the amount of bytes read and the previous limit value don't matter when
    /// calling this method.
    pub fn set_limit(&mut self, limit: u64) {
        self.limit = limit;
    }

    /// Consumes the `Take`, returning the wrapped reader.
    pub fn into_inner(self) -> T {
        self.reader
    }

    /// Gets a reference to the underlying reader.
    pub fn get_ref(&self) -> &T {
        &self.reader
    }

    /// Gets a mutable reference to the underlying reader.
    ///
    /// Care should be taken to avoid modifying the internal I/O state of the
    /// underlying reader as doing so may corrupt the internal limit of this
    /// `Take`.
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.reader
    }
}

impl<R: AsyncRead> AsyncRead for Take<R> {
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        if self.limit == 0 {
            return BufResult(Ok(0), buf);
        }

        let max = self.limit.min(buf.buf_capacity() as u64) as usize;
        let buf = buf.slice(..max);

        let (n, buf) = buf_try!(self.reader.read(buf).await.into_inner());
        assert!(n as u64 <= self.limit, "number of read bytes exceeds limit");
        self.limit -= n as u64;

        BufResult(Ok(n), buf)
    }
}

impl<R: AsyncBufRead> AsyncBufRead for Take<R> {
    async fn fill_buf(&mut self) -> IoResult<&'_ [u8]> {
        if self.limit == 0 {
            return Ok(&[]);
        }

        let buf = self.reader.fill_buf().await?;
        let cap = self.limit.min(buf.len() as u64) as usize;
        Ok(&buf[..cap])
    }

    fn consume(&mut self, amount: usize) {
        // Don't let callers reset the limit by passing an overlarge value
        let amount = self.limit.min(amount as u64) as usize;
        self.limit -= amount as u64;
        self.reader.consume(amount);
    }
}
