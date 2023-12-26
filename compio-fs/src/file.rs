use std::{future::Future, io, mem::ManuallyDrop, path::Path};

use compio_buf::{buf_try, BufResult, IntoInner, IoBuf, IoBufMut};
use compio_driver::{
    op::{BufResultExt, CloseFile, FileStat, ReadAt, Sync, WriteAt},
    syscall,
};
use compio_io::{AsyncReadAt, AsyncWriteAt};
use compio_runtime::{
    impl_attachable, impl_try_as_raw_fd, Attacher, Runtime, TryAsRawFd, TryClone,
};
#[cfg(unix)]
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
#[derive(Debug)]
pub struct File {
    inner: Attacher<std::fs::File>,
}

impl File {
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
        // Make sure that self won't be dropped after `close` called.
        // Users may call this method and drop the future immediately. In that way the
        // `close` should be cancelled.
        let this = ManuallyDrop::new(self);
        async move {
            let op = CloseFile::new(this.inner.try_as_raw_fd()?);
            Runtime::current().submit(op).await.0?;
            Ok(())
        }
    }

    /// Creates a new `File` instance that shares the same underlying file
    /// handle as the existing `File` instance.
    ///
    /// It does not clear the attach state.
    pub fn try_clone(&self) -> io::Result<Self> {
        let inner = self.inner.try_clone()?;
        Ok(Self { inner })
    }

    /// Queries metadata about the underlying file.
    pub async fn metadata(&self) -> io::Result<Metadata> {
        let op = FileStat::new(self.try_as_raw_fd()?);
        let BufResult(res, op) = Runtime::current().submit(op).await;
        res.map(|_| Metadata::from_stat(op.into_inner()))
    }

    /// Changes the permissions on the underlying file.
    #[cfg(windows)]
    pub async fn set_permissions(&self, perm: Permissions) -> io::Result<()> {
        use windows_sys::Win32::Storage::FileSystem::{
            FileBasicInfo, SetFileInformationByHandle, FILE_BASIC_INFO,
        };

        let fd = self.try_as_raw_fd()? as _;
        Runtime::current()
            .spawn_blocking(move || {
                let info = FILE_BASIC_INFO {
                    CreationTime: 0,
                    LastAccessTime: 0,
                    LastWriteTime: 0,
                    ChangeTime: 0,
                    FileAttributes: perm.0.attrs,
                };
                syscall!(
                    BOOL,
                    SetFileInformationByHandle(
                        fd,
                        FileBasicInfo,
                        &info as *const _ as _,
                        std::mem::size_of::<FILE_BASIC_INFO>() as _
                    )
                )?;
                Ok(())
            })
            .await
    }

    /// Changes the permissions on the underlying file.
    #[cfg(unix)]
    pub async fn set_permissions(&self, perm: Permissions) -> io::Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let fd = self.try_as_raw_fd()? as _;
        Runtime::current()
            .spawn_blocking(move || {
                syscall!(libc::fchmod(fd, perm.mode() as libc::mode_t))?;
                Ok(())
            })
            .await
    }

    async fn sync_impl(&self, datasync: bool) -> io::Result<()> {
        let op = Sync::new(self.try_as_raw_fd()?, datasync);
        Runtime::current().submit(op).await.0?;
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
        let (fd, buffer) = buf_try!(self.try_as_raw_fd(), buffer);
        let op = ReadAt::new(fd, pos, buffer);
        Runtime::current()
            .submit(op)
            .await
            .into_inner()
            .map_advanced()
    }

    #[cfg(unix)]
    async fn read_vectored_at<T: IoVectoredBufMut>(
        &self,
        buffer: T,
        pos: u64,
    ) -> BufResult<usize, T> {
        let (fd, buffer) = buf_try!(self.try_as_raw_fd(), buffer);
        let op = ReadVectoredAt::new(fd, pos, buffer);
        Runtime::current()
            .submit(op)
            .await
            .into_inner()
            .map_advanced()
    }
}

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

impl AsyncWriteAt for &File {
    async fn write_at<T: IoBuf>(&mut self, buffer: T, pos: u64) -> BufResult<usize, T> {
        let (fd, buffer) = buf_try!(self.try_as_raw_fd(), buffer);
        let op = WriteAt::new(fd, pos, buffer);
        Runtime::current().submit(op).await.into_inner()
    }

    #[cfg(unix)]
    async fn write_vectored_at<T: IoVectoredBuf>(
        &mut self,
        buffer: T,
        pos: u64,
    ) -> BufResult<usize, T> {
        let (fd, buffer) = buf_try!(self.try_as_raw_fd(), buffer);
        let op = WriteVectoredAt::new(fd, pos, buffer);
        Runtime::current().submit(op).await.into_inner()
    }
}

impl_try_as_raw_fd!(File, inner);

impl_attachable!(File, inner);
