use std::{future::Future, io, mem::ManuallyDrop, panic::resume_unwind, path::Path};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut};
#[cfg(unix)]
use compio_driver::op::FileStat;
use compio_driver::{
    ToSharedFd, impl_raw_fd,
    op::{BufResultExt, CloseFile, ReadAt, Sync, WriteAt},
};
use compio_io::{AsyncReadAt, AsyncWriteAt};
use compio_runtime::Attacher;
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
        let file = self.inner.clone();
        compio_runtime::spawn_blocking(move || file.metadata().map(Metadata::from_std))
            .await
            .unwrap_or_else(|e| resume_unwind(e))
    }

    /// Queries metadata about the underlying file.
    #[cfg(unix)]
    pub async fn metadata(&self) -> io::Result<Metadata> {
        let op = FileStat::new(self.to_shared_fd());
        let BufResult(res, op) = compio_runtime::submit(op).await;
        res.map(|_| Metadata::from_stat(op.into_inner()))
    }

    /// Changes the permissions on the underlying file.
    #[cfg(windows)]
    pub async fn set_permissions(&self, perm: Permissions) -> io::Result<()> {
        let file = self.inner.clone();
        compio_runtime::spawn_blocking(move || file.set_permissions(perm.0))
            .await
            .unwrap_or_else(|e| resume_unwind(e))
    }

    /// Changes the permissions on the underlying file.
    #[cfg(unix)]
    pub async fn set_permissions(&self, perm: Permissions) -> io::Result<()> {
        use std::os::unix::fs::PermissionsExt;

        use compio_driver::{AsRawFd, syscall};

        let file = self.inner.clone();
        compio_runtime::spawn_blocking(move || {
            syscall!(libc::fchmod(file.as_raw_fd(), perm.mode() as libc::mode_t))?;
            Ok(())
        })
        .await
        .unwrap_or_else(|e| resume_unwind(e))
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
        compio_runtime::submit(op).await.into_inner().map_advanced()
    }

    #[cfg(all(unix, not(solarish)))]
    async fn read_vectored_at<T: IoVectoredBufMut>(
        &self,
        buffer: T,
        pos: u64,
    ) -> BufResult<usize, T> {
        let fd = self.inner.to_shared_fd();
        let op = ReadVectoredAt::new(fd, pos, buffer);
        compio_runtime::submit(op).await.into_inner().map_advanced()
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

impl_raw_fd!(File, std::fs::File, inner, file);
