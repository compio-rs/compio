use std::{fs::Metadata, io, path::Path};

use compio_driver::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
#[cfg(feature = "runtime")]
use {
    compio_buf::{buf_try, BufResult, IntoInner, IoBuf, IoBufMut},
    compio_driver::op::{BufResultExt, ReadAt, Sync, WriteAt},
    compio_io::{AsyncReadAt, AsyncWriteAt},
    compio_runtime::{submit, Attachable, Attacher},
};

use crate::OpenOptions;

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

#[cfg(windows)]
fn file_with_options(
    path: impl AsRef<Path>,
    mut options: std::fs::OpenOptions,
) -> io::Result<std::fs::File> {
    use std::os::windows::prelude::OpenOptionsExt;

    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OVERLAPPED;

    options.custom_flags(FILE_FLAG_OVERLAPPED);
    options.open(path)
}

#[cfg(not(windows))]
fn file_with_options(
    path: impl AsRef<Path>,
    mut options: std::fs::OpenOptions,
) -> io::Result<std::fs::File> {
    use std::os::unix::prelude::OpenOptionsExt;

    // Don't set nonblocking with epoll.
    if cfg!(not(any(target_os = "linux", target_os = "android"))) {
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

    /// Creates a new `File` instance that shares the same underlying file
    /// handle as the existing `File` instance.
    ///
    /// It does not clear the attach state.
    pub fn try_clone(&self) -> io::Result<Self> {
        let inner = self.inner.try_clone()?;
        Ok(Self {
            #[cfg(feature = "runtime")]
            attacher: self.attacher.try_clone(&inner)?,
            inner,
        })
    }

    /// Queries metadata about the underlying file.
    pub fn metadata(&self) -> io::Result<Metadata> {
        self.inner.metadata()
    }

    #[cfg(feature = "runtime")]
    async fn sync_impl(&mut self, datasync: bool) -> io::Result<()> {
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
    pub async fn sync_all(&mut self) -> io::Result<()> {
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
    pub async fn sync_data(&mut self) -> io::Result<()> {
        self.sync_impl(true).await
    }
}

#[cfg(feature = "runtime")]
impl AsyncReadAt for File {
    async fn read_at<T: IoBufMut>(&self, buffer: T, pos: u64) -> BufResult<usize, T> {
        let ((), buffer) = buf_try!(self.attach(), buffer);
        let op = ReadAt::new(self.as_raw_fd(), pos, buffer);
        submit(op).await.into_inner().map_advanced()
    }
}

#[cfg(feature = "runtime")]
impl AsyncWriteAt for File {
    async fn write_at<T: IoBuf>(&mut self, buffer: T, pos: u64) -> BufResult<usize, T> {
        let ((), buffer) = buf_try!(self.attach(), buffer);
        let op = WriteAt::new(self.as_raw_fd(), pos, buffer);
        submit(op).await.into_inner()
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
            inner: FromRawFd::from_raw_fd(fd),
            #[cfg(feature = "runtime")]
            attacher: compio_runtime::Attacher::new(),
        }
    }
}

impl IntoRawFd for File {
    fn into_raw_fd(self) -> RawFd {
        self.inner.into_raw_fd()
    }
}

#[cfg(feature = "runtime")]
impl Attachable for File {
    fn attach(&self) -> io::Result<()> {
        self.attacher.attach(self)
    }

    fn is_attached(&self) -> bool {
        self.attacher.is_attached()
    }
}
