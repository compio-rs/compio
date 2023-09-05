use std::{io, path::Path};

#[cfg(feature = "runtime")]
use crate::{
    buf::{IntoInner, IoBuf, IoBufMut},
    op::{BufResultExt, ReadAt, WriteAt},
    task::RUNTIME,
    BufResult,
};
use crate::{
    driver::{fs::FileInner, AsRawFd, FromRawFd, IntoRawFd, RawFd},
    fs::OpenOptions,
};

/// A reference to an open file on the filesystem.
///
/// An instance of a `File` can be read and/or written depending on what options
/// it was opened with. The `File` type provides **positional** read and write
/// operations. The file does not maintain an internal cursor. The caller is
/// required to specify an offset when issuing an operation.
pub struct File {
    inner: FileInner,
}

impl File {
    pub(crate) fn with_options(path: impl AsRef<Path>, options: OpenOptions) -> io::Result<Self> {
        let inner = FileInner::with_options(path, options.0)?;
        #[cfg(feature = "runtime")]
        RUNTIME.with(|runtime| runtime.attach(inner.as_raw_fd()))?;
        Ok(Self { inner })
    }

    /// Attempts to open a file in read-only mode.
    ///
    /// See the [`OpenOptions::open`] method for more details.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        OpenOptions::new().read(true).open(path)
    }

    /// Opens a file in write-only mode.
    ///
    /// This function will create a file if it does not exist,
    /// and will truncate it if it does.
    ///
    /// See the [`OpenOptions::open`] function for more details.
    pub fn create(path: impl AsRef<Path>) -> io::Result<Self> {
        OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)
    }

    /// Read some bytes at the specified offset from the file into the specified
    /// buffer, returning how many bytes were read.
    ///
    /// # Return
    ///
    /// The method returns the operation result and the same buffer value passed
    /// as an argument.
    ///
    /// If the method returns [`Ok(n)`], then the read was successful. A nonzero
    /// `n` value indicates that the buffer has been filled with `n` bytes of
    /// data from the file. If `n` is `0`, then one of the following happened:
    ///
    /// 1. The specified offset is the end of the file.
    /// 2. The buffer specified was 0 bytes in capacity.
    ///
    /// It is not an error if the returned value `n` is smaller than the buffer
    /// size, even when the file contains enough data to fill the buffer.
    ///
    /// # Errors
    ///
    /// If this function encounters any form of I/O or other error, an error
    /// variant will be returned. The buffer is returned on error.
    #[cfg(feature = "runtime")]
    pub async fn read_at<T: IoBufMut>(&self, buffer: T, pos: usize) -> BufResult<usize, T> {
        let op = ReadAt::new(self.as_raw_fd(), pos, buffer);
        RUNTIME
            .with(|runtime| runtime.submit(op))
            .await
            .into_inner()
            .map_advanced()
            .into_inner()
    }

    /// Write a buffer into this file at the specified offset, returning how
    /// many bytes were written.
    ///
    /// This function will attempt to write the entire contents of `buf`, but
    /// the entire write may not succeed, or the write may also generate an
    /// error. The bytes will be written starting at the specified offset.
    ///
    /// # Return
    ///
    /// The method returns the operation result and the same buffer value passed
    /// in as an argument. A return value of `0` typically means that the
    /// underlying file is no longer able to accept bytes and will likely not be
    /// able to in the future as well, or that the buffer provided is empty.
    ///
    /// # Errors
    ///
    /// Each call to `write_at` may generate an I/O error indicating that the
    /// operation could not be completed. If an error is returned then no bytes
    /// in the buffer were written to this writer.
    ///
    /// It is **not** considered an error if the entire buffer could not be
    /// written to this writer.
    #[cfg(feature = "runtime")]
    pub async fn write_at<T: IoBuf>(&self, buffer: T, pos: usize) -> BufResult<usize, T> {
        let op = WriteAt::new(self.as_raw_fd(), pos, buffer);
        RUNTIME
            .with(|runtime| runtime.submit(op))
            .await
            .into_inner()
            .into_inner()
    }
}

impl AsRawFd for File {
    fn as_raw_fd(&self) -> RawFd {
        self.inner.as_raw_fd()
    }
}

impl FromRawFd for File {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self {
            inner: FileInner::from_raw_fd(fd),
        }
    }
}

impl IntoRawFd for File {
    fn into_raw_fd(self) -> RawFd {
        self.inner.into_raw_fd()
    }
}
