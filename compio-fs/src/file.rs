use std::{fs::Metadata, io, path::Path};

use compio_driver::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
#[cfg(feature = "runtime")]
use {
    compio_buf::{buf_try, BufResult, IntoInner, IoBuf, IoBufMut},
    compio_driver::op::{BufResultExt, CloseFile, ReadAt, Sync, WriteAt},
    compio_io::{AsyncReadAt, AsyncWriteAt},
    compio_runtime::{submit, Attachable, Attacher},
    std::{future::Future, mem::ManuallyDrop},
};
#[cfg(all(feature = "runtime", unix))]
use {
    compio_buf::{IoVectoredBuf, IoVectoredBufMut},
    compio_driver::op::{ReadVectoredAt, WriteVectoredAt},
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

impl File {
    /// Attempts to open a file in read-only mode.
    ///
    /// See the [`OpenOptions::open`] method for more details.
    #[cfg(feature = "runtime")]
    pub async fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        OpenOptions::new().read(true).open(path).await
    }

    /// Opens a file in write-only mode.
    ///
    /// This function will create a file if it does not exist,
    /// and will truncate it if it does.
    ///
    /// See the [`OpenOptions::open`] function for more details.
    #[cfg(feature = "runtime")]
    pub async fn create(path: impl AsRef<Path>) -> io::Result<Self> {
        OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)
            .await
    }

    /// Close the file. If the returned future is dropped before polling, the
    /// file won't be closed.
    #[cfg(feature = "runtime")]
    pub fn close(self) -> impl Future<Output = io::Result<()>> {
        // Make sure that self won't be dropped after `close` called.
        // Users may call this method and drop the future immediately. In that way the
        // `close` should be cancelled.
        let this = ManuallyDrop::new(self);
        async move {
            let op = CloseFile::new(this.as_raw_fd());
            submit(op).await.0?;
            Ok(())
        }
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

#[cfg(feature = "runtime")]
impl AsyncReadAt for File {
    async fn read_at<T: IoBufMut>(&self, buffer: T, pos: u64) -> BufResult<usize, T> {
        let ((), buffer) = buf_try!(self.attach(), buffer);
        let op = ReadAt::new(self.as_raw_fd(), pos, buffer);
        submit(op).await.into_inner().map_advanced()
    }

    #[cfg(unix)]
    async fn read_vectored_at<T: IoVectoredBufMut>(
        &self,
        buffer: T,
        pos: u64,
    ) -> BufResult<usize, T> {
        let ((), buffer) = buf_try!(self.attach(), buffer);
        let op = ReadVectoredAt::new(self.as_raw_fd(), pos, buffer);
        submit(op).await.into_inner().map_advanced()
    }
}

#[cfg(feature = "runtime")]
impl AsyncWriteAt for File {
    #[inline]
    async fn write_at<T: IoBuf>(&mut self, buf: T, pos: u64) -> BufResult<usize, T> {
        (&*self).write_at(buf, pos).await
    }

    #[cfg(unix)]
    #[inline]
    async fn write_vectored_at<T: IoVectoredBuf>(
        &mut self,
        buf: T,
        pos: u64,
    ) -> BufResult<usize, T> {
        (&*self).write_vectored_at(buf, pos).await
    }
}

#[cfg(feature = "runtime")]
impl AsyncWriteAt for &File {
    async fn write_at<T: IoBuf>(&mut self, buffer: T, pos: u64) -> BufResult<usize, T> {
        let ((), buffer) = buf_try!(self.attach(), buffer);
        let op = WriteAt::new(self.as_raw_fd(), pos, buffer);
        submit(op).await.into_inner()
    }

    #[cfg(unix)]
    async fn write_vectored_at<T: IoVectoredBuf>(
        &mut self,
        buffer: T,
        pos: u64,
    ) -> BufResult<usize, T> {
        let ((), buffer) = buf_try!(self.attach(), buffer);
        let op = WriteVectoredAt::new(self.as_raw_fd(), pos, buffer);
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
