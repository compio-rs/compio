use std::{future::Future, io, mem::ManuallyDrop, path::Path};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut};
#[cfg(unix)]
use compio_driver::op::FileStat;
use compio_driver::{
    ToSharedFd, impl_raw_fd,
    op::{
        AsyncifyFd, BufResultExt, CloseFile, ReadAt, ReadManagedAt, ResultTakeBuffer, Sync, WriteAt,
    },
};
use compio_io::{AsyncReadAt, AsyncReadManagedAt, AsyncWriteAt, util::Splittable};
use compio_runtime::{Attacher, BorrowedBuffer, BufferPool};
#[cfg(all(unix, not(solarish)))]
use {
    compio_buf::{IoVectoredBuf, IoVectoredBufMut},
    compio_driver::op::{ReadVectoredAt, WriteVectoredAt},
};

use crate::{Metadata, OpenOptions, Permissions};

/// A reference to an open file on the filesystem.
///
/// An instance of a `File` can be read and/or written depending on what options
/// it was opened with. The `File` type provides **positional** read and write
/// operations. The file does not maintain an internal cursor. The caller is
/// required to specify an offset when issuing an operation.
///
///
/// If you'd like to use methods from [`AsyncRead`](`compio_io::AsyncRead`) or
/// [`AsyncWrite`](`compio_io::AsyncWrite`) traits, you can wrap `File` with
/// [`std::io::Cursor`].
///
/// # Examples
/// ```ignore
/// use compio::fs::File;
/// use compio::buf::BufResult;
/// use std::io::Cursor;
///
/// let file = File::open("foo.txt").await?;
/// let cursor = Cursor::new(file);
///
/// let int = cursor.read_u32().await?;
/// let float = cursor.read_f32().await?;
///
/// let mut string = String::new();
/// let BufResult(result, string) = cursor.read_to_string(string).await;
///
/// let mut buf = vec![0; 1024];
/// let BufResult(result, buf) = cursor.read_exact(buf).await;
/// ```
#[derive(Debug, Clone)]
pub struct File {
    inner: Attacher<std::fs::File>,
}

impl File {
    pub(crate) fn from_std(file: std::fs::File) -> io::Result<Self> {
        Ok(Self {
            inner: Attacher::new(file)?,
        })
    }

    /// Attempts to open a file in read-only mode.
    ///
    /// See the [`OpenOptions::open`] method for more details.
    pub async fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        OpenOptions::new().read(true).open(path).await
    }

    /// Opens a file in write-only mode.
    ///
    /// This function will create a file if it does not exist,
    /// and will truncate it if it does.
    ///
    /// See the [`OpenOptions::open`] function for more details.
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
    pub fn close(self) -> impl Future<Output = io::Result<()>> {
        // Make sure that fd won't be dropped after `close` called.
        // Users may call this method and drop the future immediately. In that way
        // `close` should be cancelled.
        let this = ManuallyDrop::new(self);
        async move {
            let fd = ManuallyDrop::into_inner(this)
                .inner
                .into_inner()
                .take()
                .await;
            if let Some(fd) = fd {
                let op = CloseFile::new(fd.into());
                compio_runtime::submit(op).await.0?;
            }
            Ok(())
        }
    }

    /// Queries metadata about the underlying file.
    #[cfg(windows)]
    pub async fn metadata(&self) -> io::Result<Metadata> {
        let op = AsyncifyFd::new(self.to_shared_fd(), |file: &std::fs::File| {
            match file.metadata().map(Metadata::from_std) {
                Ok(meta) => BufResult(Ok(0), Some(meta)),
                Err(e) => BufResult(Err(e), None),
            }
        });
        let BufResult(res, meta) = compio_runtime::submit(op).await;
        res.map(|_| meta.into_inner().expect("metadata should be present"))
    }

    /// Queries metadata about the underlying file.
    #[cfg(unix)]
    pub async fn metadata(&self) -> io::Result<Metadata> {
        let op = FileStat::new(self.to_shared_fd());
        let BufResult(res, op) = compio_runtime::submit(op).await;
        res.map(|_| Metadata::from_stat(op.into_inner()))
    }

    /// Changes the permissions on the underlying file.
    pub async fn set_permissions(&self, perm: Permissions) -> io::Result<()> {
        let op = AsyncifyFd::new(self.to_shared_fd(), move |file: &std::fs::File| {
            BufResult(file.set_permissions(perm.0).map(|_| 0), ())
        });
        compio_runtime::submit(op).await.0.map(|_| ())
    }

    async fn sync_impl(&self, datasync: bool) -> io::Result<()> {
        let op = Sync::new(self.to_shared_fd(), datasync);
        compio_runtime::submit(op).await.0?;
        Ok(())
    }

    /// Attempts to sync all OS-internal metadata to disk.
    ///
    /// This function will attempt to ensure that all in-memory data reaches the
    /// filesystem before returning.
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
    pub async fn sync_data(&self) -> io::Result<()> {
        self.sync_impl(true).await
    }
}

impl AsyncReadAt for File {
    async fn read_at<T: IoBufMut>(&self, buffer: T, pos: u64) -> BufResult<usize, T> {
        let fd = self.inner.to_shared_fd();
        let op = ReadAt::new(fd, pos, buffer);
        let res = compio_runtime::submit(op).await.into_inner();
        unsafe { res.map_advanced() }
    }

    #[cfg(all(unix, not(solarish)))]
    async fn read_vectored_at<T: IoVectoredBufMut>(
        &self,
        buffer: T,
        pos: u64,
    ) -> BufResult<usize, T> {
        use compio_driver::op::VecBufResultExt;

        let fd = self.inner.to_shared_fd();
        let op = ReadVectoredAt::new(fd, pos, buffer);
        let res = compio_runtime::submit(op).await.into_inner();
        unsafe { res.map_vec_advanced() }
    }
}

impl AsyncReadManagedAt for File {
    type Buffer<'a> = BorrowedBuffer<'a>;
    type BufferPool = BufferPool;

    async fn read_managed_at<'a>(
        &self,
        buffer_pool: &'a Self::BufferPool,
        len: usize,
        pos: u64,
    ) -> io::Result<Self::Buffer<'a>> {
        let fd = self.inner.to_shared_fd();
        let buffer_pool = buffer_pool.try_inner()?;
        let op = ReadManagedAt::new(fd, pos, buffer_pool, len)?;
        compio_runtime::submit_with_extra(op)
            .await
            .take_buffer(buffer_pool)
    }
}

impl AsyncWriteAt for File {
    #[inline]
    async fn write_at<T: IoBuf>(&mut self, buf: T, pos: u64) -> BufResult<usize, T> {
        (&*self).write_at(buf, pos).await
    }

    #[cfg(all(unix, not(solarish)))]
    #[inline]
    async fn write_vectored_at<T: IoVectoredBuf>(
        &mut self,
        buf: T,
        pos: u64,
    ) -> BufResult<usize, T> {
        (&*self).write_vectored_at(buf, pos).await
    }
}

impl AsyncWriteAt for &File {
    async fn write_at<T: IoBuf>(&mut self, buffer: T, pos: u64) -> BufResult<usize, T> {
        let fd = self.inner.to_shared_fd();
        let op = WriteAt::new(fd, pos, buffer);
        compio_runtime::submit(op).await.into_inner()
    }

    #[cfg(all(unix, not(solarish)))]
    async fn write_vectored_at<T: IoVectoredBuf>(
        &mut self,
        buffer: T,
        pos: u64,
    ) -> BufResult<usize, T> {
        let fd = self.inner.to_shared_fd();
        let op = WriteVectoredAt::new(fd, pos, buffer);
        compio_runtime::submit(op).await.into_inner()
    }
}

impl Splittable for File {
    type ReadHalf = File;
    type WriteHalf = File;

    fn split(self) -> (Self::ReadHalf, Self::WriteHalf) {
        (self.clone(), self)
    }
}

impl Splittable for &File {
    type ReadHalf = File;
    type WriteHalf = File;

    fn split(self) -> (Self::ReadHalf, Self::WriteHalf) {
        (self.clone(), self.clone())
    }
}

impl_raw_fd!(File, std::fs::File, inner, file);
