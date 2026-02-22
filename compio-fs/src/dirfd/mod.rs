use std::{io, path::Path};

use compio_buf::{BufResult, IoBuf, buf_try};
use compio_io::{AsyncReadAtExt, AsyncWriteAtExt};

use crate::{DirBuilder, File, Metadata, OpenOptions};

#[cfg(unix)]
#[path = "unix.rs"]
mod sys;

#[cfg(windows)]
#[path = "windows.rs"]
mod sys;

/// A reference to an open directory on a filesystem.
#[derive(Debug, Clone)]
pub struct Dir {
    inner: sys::Dir,
}

impl Dir {
    /// Opens a directory at the specified path and returns a reference to it.
    pub async fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        Ok(Dir {
            inner: sys::Dir::open(path).await?,
        })
    }

    /// Opens a file at `path` with the options specified by `options`.
    pub async fn open_file_with(
        &self,
        path: impl AsRef<Path>,
        options: &OpenOptions,
    ) -> io::Result<File> {
        self.inner.open_file_with(path, options).await
    }

    /// Attempts to open a file in read-only mode.
    pub async fn open_file(&self, path: impl AsRef<Path>) -> io::Result<File> {
        self.open_file_with(path, OpenOptions::new().read(true))
            .await
    }

    /// Opens a file in write-only mode.
    pub async fn create_file(&self, path: impl AsRef<Path>) -> io::Result<File> {
        self.open_file_with(
            path,
            OpenOptions::new().write(true).create(true).truncate(true),
        )
        .await
    }

    /// Attempts to open a directory.
    pub async fn open_dir(&self, path: impl AsRef<Path>) -> io::Result<Self> {
        Ok(Self {
            inner: self.inner.open_dir(path).await?,
        })
    }

    /// Creates the specified directory with the options configured in this
    /// builder.
    pub async fn create_dir_with(
        &self,
        path: impl AsRef<Path>,
        builder: &DirBuilder,
    ) -> io::Result<()> {
        self.inner.create_dir_with(path, builder).await
    }

    /// Creates a new, empty directory at the provided path.
    pub async fn create_dir(&self, path: impl AsRef<Path>) -> io::Result<()> {
        self.create_dir_with(path, &DirBuilder::new()).await
    }

    /// Recursively create a directory and all of its parent components if they
    /// are missing.
    pub async fn create_dir_all(&self, path: impl AsRef<Path>) -> io::Result<()> {
        self.create_dir_with(path, DirBuilder::new().recursive(true))
            .await
    }

    /// Queries metadata about the underlying directory.
    pub async fn dir_metadata(&self) -> io::Result<Metadata> {
        self.inner.dir_metadata().await
    }

    /// Given a path, query the file system to get information about a file,
    /// directory, etc.
    pub async fn metadata(&self, path: impl AsRef<Path>) -> io::Result<Metadata> {
        self.inner.metadata(path).await
    }

    /// Query the metadata about a file without following symlinks.
    pub async fn symlink_metadata(&self, path: impl AsRef<Path>) -> io::Result<Metadata> {
        self.inner.symlink_metadata(path).await
    }

    /// Creates a new hard link on a filesystem.
    pub async fn hard_link(
        &self,
        source: impl AsRef<Path>,
        target_dir: &Self,
        target: impl AsRef<Path>,
    ) -> io::Result<()> {
        self.inner
            .hard_link(source, &target_dir.inner, target)
            .await
    }

    /// Creates a new symbolic link on a filesystem.
    ///
    /// The `original` argument provides the target of the symlink. The `link`
    /// argument provides the name of the created symlink.
    #[cfg(unix)]
    pub async fn symlink(
        &self,
        original: impl AsRef<Path>,
        link: impl AsRef<Path>,
    ) -> io::Result<()> {
        self.inner.symlink(original, link).await
    }

    /// Rename a file or directory to a new name, replacing the original file if
    /// to already exists.
    pub async fn rename(
        &self,
        from: impl AsRef<Path>,
        to_dir: &Self,
        to: impl AsRef<Path>,
    ) -> io::Result<()> {
        self.inner.rename(from, &to_dir.inner, to).await
    }

    /// Removes a file from a filesystem.
    pub async fn remove_file(&self, path: impl AsRef<Path>) -> io::Result<()> {
        self.inner.remove_file(path).await
    }

    /// Removes an empty directory.
    pub async fn remove_dir(&self, path: impl AsRef<Path>) -> io::Result<()> {
        self.inner.remove_dir(path).await
    }

    /// Read the entire contents of a file into a bytes vector.
    pub async fn read(&self, path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
        let file = self.open_file(path).await?;
        let BufResult(res, buf) = file.read_to_end_at(Vec::new(), 0).await;
        res?;
        Ok(buf)
    }

    /// Write a buffer as the entire contents of a file.
    pub async fn write<B: IoBuf>(&self, path: impl AsRef<Path>, buf: B) -> BufResult<(), B> {
        let (mut file, buf) = buf_try!(self.create_file(path).await, buf);
        file.write_all_at(buf, 0).await
    }
}

compio_driver::impl_raw_fd!(Dir, std::fs::File, inner);
