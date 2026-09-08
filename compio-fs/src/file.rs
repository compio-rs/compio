use std::{future::Future, io, mem::ManuallyDrop, path::Path};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut};
#[cfg(unix)]
use compio_driver::op::FileStat;
use compio_driver::{
    BufferRef, ResultTakeBuffer, ToSharedFd, impl_raw_fd,
    op::{BufResultExt, CloseFile, ReadAt, ReadManagedAt, Sync, WriteAt},
};
use compio_io::{AsyncReadAt, AsyncReadManagedAt, AsyncWriteAt, util::Splittable};
use compio_runtime::{Runtime, fd::AsyncFd};
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
    pub(crate) inner: AsyncFd<std::fs::File>,
}

impl File {
    pub(crate) fn from_std(file: std::fs::File) -> io::Result<Self> {
        Ok(Self {
            inner: AsyncFd::new(file)?,
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
    ///
    /// As [`File`] is clonable, users can call `close` on a clone, but the
    /// future will never complete until all clones are dropped. Some
    /// operations may keep a strong reference to the file, so the future
    /// may never complete if there are pending operations.
    ///
    /// It's OK to drop the [`File`] directly without calling `close`, but the
    /// file may not be closed immediately.
    pub fn close(self) -> impl Future<Output = io::Result<()>> {
        // Make sure that fd won't be dropped after `close` called.
        // Users may call this method and drop the future immediately. In that
        // way `close` should be cancelled.
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
        crate::spawn_blocking_with(self.to_shared_fd(), |file| {
            file.metadata().map(Metadata::from_std)
        })
        .await
    }

    #[cfg(windows)]
    /// Truncates or extends the underlying file, updating the size of this file
    /// to become `size`.
    pub async fn set_len(&self, size: u64) -> io::Result<()> {
        crate::spawn_blocking_with(self.to_shared_fd(), move |file| file.set_len(size)).await
    }

    #[cfg(unix)]
    /// Truncates or extends the underlying file, updating the size of this file
    /// to become `size`.
    ///
    /// NOTE: On Linux kernel <= 6.9 or when io uring is disabled, the operation
    /// will be offloaded to the separate blocking thread
    pub async fn set_len(&self, size: u64) -> io::Result<()> {
        use compio_driver::op::TruncateFile;

        let op = TruncateFile::new(self.to_shared_fd(), size);
        compio_runtime::submit(op).await.0.map(|_| ())
    }

    /// Queries metadata about the underlying file.
    #[cfg(unix)]
    pub async fn metadata(&self) -> io::Result<Metadata> {
        let op = FileStat::new(self.to_shared_fd());
        let BufResult(res, op) = compio_runtime::submit(op).await;
        res.map(|_| Metadata::from_attr(op.into_inner()))
    }

    /// Reads an extended attribute from this open file, with `fgetxattr`
    /// semantics.
    ///
    /// The file descriptor identifies the object; no pathname is resolved
    /// again. In particular, this does not read attributes from a symbolic
    /// link itself.
    ///
    /// The value is written from the start of `buffer`, using its full
    /// capacity. On success, the result is the value's length and the
    /// buffer's initialized length advances to at least that length. With
    /// zero capacity, only the required length is returned and the buffer
    /// remains empty. The attribute may change between a sizing query and a
    /// subsequent read.
    ///
    /// A missing attribute returns `ENODATA`; insufficient nonzero capacity
    /// returns `ERANGE`. Other OS errors are preserved. A name containing a NUL
    /// byte returns [`io::ErrorKind::InvalidInput`]. Errors return the original
    /// buffer without advancing its initialized length.
    ///
    /// The submitted operation owns the name and buffer and holds a reference
    /// to the file descriptor until completion, even if this future is
    /// dropped. Cancellation does not return the buffer to the caller.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    pub async fn get_xattr<T: IoBufMut>(
        &self,
        name: impl AsRef<std::ffi::OsStr>,
        buffer: T,
    ) -> BufResult<usize, T> {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};

        use compio_buf::{IoBufMutExt, buf_try};
        use compio_driver::op::FGetXattr;

        let (name, mut buffer) = buf_try!(
            CString::new(name.as_ref().as_bytes()).map_err(io::Error::from),
            buffer
        );
        let query_size = buffer.buf_capacity() == 0;
        let op = FGetXattr::new(self.to_shared_fd(), name, buffer);
        let res = compio_runtime::submit(op).await.into_inner();
        if query_size {
            res
        } else {
            // SAFETY: A successful nonzero-capacity fgetxattr initializes
            // exactly the returned byte count, bounded by the
            // supplied buffer capacity. Size-only queries are
            // excluded because they do not write bytes.
            unsafe { res.map_advanced() }
        }
    }

    /// Changes the permissions on the underlying file.
    #[cfg(windows)]
    pub async fn set_permissions(&self, perm: Permissions) -> io::Result<()> {
        crate::spawn_blocking_with(self.to_shared_fd(), move |file| {
            if let Some(p) = perm.0.original {
                file.set_permissions(p)
            } else {
                let mut p = file.metadata()?.permissions();
                p.set_readonly(perm.readonly());
                file.set_permissions(p)
            }
        })
        .await
    }

    /// Changes the permissions on the underlying file.
    #[cfg(unix)]
    pub async fn set_permissions(&self, perm: Permissions) -> io::Result<()> {
        crate::spawn_blocking_with(self.to_shared_fd(), |file| file.set_permissions(perm.0)).await
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
    type Buffer = BufferRef;

    async fn read_managed_at(&self, len: usize, pos: u64) -> io::Result<Option<Self::Buffer>> {
        let fd = self.inner.to_shared_fd();
        let res = Runtime::with_current(|rt| {
            let buffer_pool = rt.buffer_pool()?;
            let op = ReadManagedAt::new(fd, pos, &buffer_pool, len)?;
            io::Result::Ok(rt.submit(op))
        })?
        .await;
        unsafe { res.take_buffer() }
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

impl Splittable for &mut File {
    type ReadHalf = File;
    type WriteHalf = File;

    fn split(self) -> (Self::ReadHalf, Self::WriteHalf) {
        (self.clone(), self.clone())
    }
}

impl_raw_fd!(File, std::fs::File, inner, file);
