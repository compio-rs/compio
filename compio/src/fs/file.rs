#[cfg(feature = "allocator_api")]
use std::alloc::Allocator;
use std::{fs::Metadata, io, path::Path};

#[cfg(feature = "runtime")]
use crate::{
    buf::{vec_alloc, IntoInner, IoBuf, IoBufMut, SetBufInit},
    buf_try,
    driver::AsRawFd,
    op::{BufResultExt, ReadAt, Sync, WriteAt},
    task::submit,
    Attacher, BufResult,
};
use crate::{fs::OpenOptions, impl_raw_fd};

/// A reference to an open file on the filesystem.
///
/// An instance of a `File` can be read and/or written depending on what options
/// it was opened with. The `File` type provides **positional** read and write
/// operations. The file does not maintain an internal cursor. The caller is
/// required to specify an offset when issuing an operation.
#[derive(Debug)]
pub struct File {
    inner: std::fs::File,
    #[cfg(feature = "runtime")]
    attacher: Attacher,
}

#[cfg(target_os = "windows")]
fn file_with_options(
    path: impl AsRef<Path>,
    mut options: std::fs::OpenOptions,
) -> io::Result<std::fs::File> {
    use std::os::windows::prelude::OpenOptionsExt;

    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OVERLAPPED;

    options.custom_flags(FILE_FLAG_OVERLAPPED);
    options.open(path)
}

#[cfg(not(target_os = "windows"))]
fn file_with_options(
    path: impl AsRef<Path>,
    mut options: std::fs::OpenOptions,
) -> io::Result<std::fs::File> {
    use std::os::unix::prelude::OpenOptionsExt;

    // Don't set nonblocking with epoll.
    if cfg!(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "illumos"
    ))) {
        options.custom_flags(libc::O_NONBLOCK);
    }
    options.open(path)
}

impl File {
    pub(crate) fn with_options(path: impl AsRef<Path>, options: OpenOptions) -> io::Result<Self> {
        let this = Self {
            inner: file_with_options(path, options.0)?,
            #[cfg(feature = "runtime")]
            attacher: Attacher::new(),
        };
        Ok(this)
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

    #[cfg(feature = "runtime")]
    pub(crate) fn attach(&self) -> io::Result<()> {
        self.attacher.attach(self)
    }

    /// Creates a new `File` instance that shares the same underlying file
    /// handle as the existing `File` instance.
    ///
    /// It does not clear the attach state.
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            inner: self.inner.try_clone()?,
            #[cfg(feature = "runtime")]
            attacher: self.attacher.clone(),
        })
    }

    /// Queries metadata about the underlying file.
    pub fn metadata(&self) -> io::Result<Metadata> {
        self.inner.metadata()
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
    pub async fn read_at<T: IoBufMut + SetBufInit>(
        &self,
        buffer: T,
        pos: usize,
    ) -> BufResult<usize, T> {
        let ((), buffer) = buf_try!(self.attach(), buffer);
        let op = ReadAt::new(self.as_raw_fd(), pos, buffer);
        submit(op).await.into_inner().map_advanced()
    }

    /// Read the exact number of bytes required to fill `buffer`.
    ///
    /// This function reads as many bytes as necessary to completely fill the
    /// uninitialized space of specified `buffer`.
    ///
    /// # Errors
    ///
    /// If this function encounters an "end of file" before completely filling
    /// the buffer, it returns an error of the kind
    /// [`ErrorKind::UnexpectedEof`]. The contents of `buffer` are unspecified
    /// in this case.
    ///
    /// If any other read error is encountered then this function immediately
    /// returns. The contents of `buffer` are unspecified in this case.
    ///
    /// If this function returns an error, it is unspecified how many bytes it
    /// has read, but it will never read more than would be necessary to
    /// completely fill the buffer.
    ///
    /// [`ErrorKind::UnexpectedEof`]: io::ErrorKind::UnexpectedEof
    #[cfg(feature = "runtime")]
    pub async fn read_exact_at<T: IoBufMut + SetBufInit>(
        &self,
        mut buffer: T,
        pos: usize,
    ) -> BufResult<usize, T> {
        let need = buffer.as_uninit_slice().len();
        let mut total_read = 0;
        let mut read;
        while total_read < need {
            (read, buffer) = buf_try!(self.read_at(buffer, pos + total_read).await);
            if read == 0 {
                break;
            } else {
                total_read += read;
            }
        }
        let res = if total_read < need {
            Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "failed to fill whole buffer",
            ))
        } else {
            Ok(total_read)
        };
        (res, buffer)
    }

    /// Read all bytes until EOF in this source, placing them into `buffer`.
    ///
    /// All bytes read from this source will be appended to the specified buffer
    /// `buffer`. This function will continuously call [`read_at()`] to append
    /// more data to `buffer` until [`read_at()`] returns [`Ok(0)`].
    ///
    /// If successful, this function will return the total number of bytes read.
    ///
    /// [`read_at()`]: File::read_at
    #[cfg(feature = "runtime")]
    pub async fn read_to_end_at<
        #[cfg(feature = "allocator_api")] A: Allocator + Unpin + 'static,
    >(
        &self,
        mut buffer: vec_alloc!(u8, A),
        pos: usize,
    ) -> BufResult<usize, vec_alloc!(u8, A)> {
        let mut total_read = 0;
        let mut read;
        loop {
            (read, buffer) = buf_try!(self.read_at(buffer, pos + total_read).await);
            if read == 0 {
                break;
            } else {
                total_read += read;
                if buffer.len() == buffer.capacity() {
                    buffer.reserve(32);
                }
            }
        }
        (Ok(total_read), buffer)
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
        let ((), buffer) = buf_try!(self.attach(), buffer);
        let op = WriteAt::new(self.as_raw_fd(), pos, buffer);
        submit(op).await.into_inner()
    }

    /// Attempts to write an entire buffer into this writer.
    ///
    /// This method will continuously call [`write_at`] until there is no more
    /// data to be written. This method will not return until the entire
    /// buffer has been successfully written or such an error occurs.
    ///
    /// If the buffer contains no data, this will never call [`write_at`].
    ///
    /// [`write_at`]: File::write_at
    #[cfg(feature = "runtime")]
    pub async fn write_all_at<T: IoBuf>(&self, mut buffer: T, pos: usize) -> BufResult<usize, T> {
        let buf_len = buffer.buf_len();
        let mut total_written = 0;
        let mut written;
        while total_written < buf_len {
            (written, buffer) = buf_try!(
                self.write_at(buffer.slice(total_written..), pos + total_written)
                    .await
                    .into_inner()
            );
            total_written += written;
        }
        (Ok(total_written), buffer)
    }

    #[cfg(feature = "runtime")]
    async fn sync_impl(&self, datasync: bool) -> io::Result<()> {
        self.attach()?;
        let op = Sync::new(self.as_raw_fd(), datasync);
        submit(op).await.0?;
        Ok(())
    }

    /// Attempts to sync all OS-internal metadata to disk.
    ///
    /// This function will attempt to ensure that all in-memory data reaches the
    /// filesystem before returning.
    #[cfg(feature = "runtime")]
    pub async fn sync_all(&self) -> io::Result<()> {
        self.sync_impl(false).await
    }

    /// This function is similar to [`sync_all`], except that it might not
    /// synchronize file metadata to the filesystem.
    ///
    /// This is intended for use cases that must synchronize content, but don't
    /// need the metadata on disk. The goal of this method is to reduce disk
    /// operations.
    ///
    /// Note that some platforms may simply implement this in terms of
    /// [`sync_all`].
    ///
    /// [`sync_all`]: File::sync_all
    #[cfg(feature = "runtime")]
    pub async fn sync_data(&self) -> io::Result<()> {
        self.sync_impl(true).await
    }
}

impl_raw_fd!(File, inner, attacher);
