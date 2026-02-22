#[cfg(unix)]
#[path = "unix.rs"]
mod sys;

#[cfg(windows)]
#[path = "windows.rs"]
mod sys;

use std::{io, path::Path};

use compio_buf::{BufResult, IoBuf, buf_try};
use compio_io::{AsyncReadAtExt, AsyncWriteAtExt};

use crate::{File, metadata};

/// Removes a file from the filesystem.
pub async fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
    sys::remove_file(path).await
}

/// Removes an empty directory.
pub async fn remove_dir(path: impl AsRef<Path>) -> io::Result<()> {
    sys::remove_dir(path).await
}

/// Creates a new, empty directory at the provided path.
pub async fn create_dir(path: impl AsRef<Path>) -> io::Result<()> {
    DirBuilder::new().create(path).await
}

/// Recursively create a directory and all of its parent components if they are
/// missing.
pub async fn create_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
    DirBuilder::new().recursive(true).create(path).await
}

/// Rename a file or directory to a new name, replacing the original file if
/// `to` already exists.
pub async fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
    sys::rename(from, to).await
}

/// Creates a new symbolic link on the filesystem.
#[cfg(unix)]
pub async fn symlink(original: impl AsRef<Path>, link: impl AsRef<Path>) -> io::Result<()> {
    sys::symlink(original, link).await
}

/// Creates a new symlink to a non-directory file on the filesystem.
#[cfg(windows)]
pub async fn symlink_file(original: impl AsRef<Path>, link: impl AsRef<Path>) -> io::Result<()> {
    sys::symlink_file(original, link).await
}

/// Creates a new symlink to a directory on the filesystem.
#[cfg(windows)]
pub async fn symlink_dir(original: impl AsRef<Path>, link: impl AsRef<Path>) -> io::Result<()> {
    sys::symlink_dir(original, link).await
}

/// Creates a new hard link on the filesystem.
pub async fn hard_link(original: impl AsRef<Path>, link: impl AsRef<Path>) -> io::Result<()> {
    sys::hard_link(original, link).await
}

/// Write a slice as the entire contents of a file.
///
/// This function will create a file if it does not exist,
/// and will entirely replace its contents if it does.
pub async fn write<P: AsRef<Path>, B: IoBuf>(path: P, buf: B) -> BufResult<(), B> {
    let (mut file, buf) = buf_try!(File::create(path).await, buf);
    file.write_all_at(buf, 0).await
}

/// Read the entire contents of a file into a bytes vector.
pub async fn read<P: AsRef<Path>>(path: P) -> io::Result<Vec<u8>> {
    let file = File::open(path).await?;
    let BufResult(res, buffer) = file.read_to_end_at(Vec::new(), 0).await;
    res?;
    Ok(buffer)
}

/// A builder used to create directories in various manners.
pub struct DirBuilder {
    inner: sys::DirBuilder,
    recursive: bool,
}

impl Default for DirBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl DirBuilder {
    /// Creates a new set of options with default mode/security settings for all
    /// platforms and also non-recursive.
    pub fn new() -> Self {
        Self {
            inner: sys::DirBuilder::new(),
            recursive: false,
        }
    }

    /// Indicates that directories should be created recursively, creating all
    /// parent directories. Parents that do not exist are created with the same
    /// security and permissions settings.
    pub fn recursive(&mut self, recursive: bool) -> &mut Self {
        self.recursive = recursive;
        self
    }

    /// Creates the specified directory with the options configured in this
    /// builder.
    pub async fn create(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let path = path.as_ref();
        if self.recursive {
            self.create_dir_all(path).await
        } else {
            self.inner.create(path).await
        }
    }

    async fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        if path == Path::new("") {
            return Ok(());
        }

        match self.inner.create(path).await {
            Ok(()) => return Ok(()),
            Err(ref e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(_) if metadata(path).await.map(|m| m.is_dir()).unwrap_or_default() => return Ok(()),
            Err(e) => return Err(e),
        }
        match path.parent() {
            Some(p) => Box::pin(self.create_dir_all(p)).await?,
            None => {
                return Err(io::Error::other("failed to create whole tree"));
            }
        }
        match self.inner.create(path).await {
            Ok(()) => Ok(()),
            Err(_) if metadata(path).await.map(|m| m.is_dir()).unwrap_or_default() => Ok(()),
            Err(e) => Err(e),
        }
    }

    #[cfg(unix)]
    pub(crate) async fn create_at(&self, dir: &File, path: &Path) -> io::Result<()> {
        if path.is_absolute() {
            self.create(path).await
        } else if self.recursive {
            self.create_dir_all_at(dir, path).await
        } else {
            self.inner.create_at(dir, path).await
        }
    }

    #[cfg(unix)]
    async fn create_dir_all_at(&self, dir: &File, path: &Path) -> io::Result<()> {
        use crate::metadata_at;

        if path == Path::new("") {
            return Ok(());
        }
        match self.inner.create_at(dir, path).await {
            Ok(()) => return Ok(()),
            Err(ref e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(_)
                if metadata_at(dir, path)
                    .await
                    .map(|m| m.is_dir())
                    .unwrap_or_default() =>
            {
                return Ok(());
            }
            Err(e) => return Err(e),
        }
        match path.parent() {
            Some(p) => Box::pin(self.create_dir_all_at(dir, p)).await?,
            None => {
                return Err(io::Error::other("failed to create whole tree"));
            }
        }
        match self.inner.create_at(dir, path).await {
            Ok(()) => Ok(()),
            Err(_)
                if metadata_at(dir, path)
                    .await
                    .map(|m| m.is_dir())
                    .unwrap_or_default() =>
            {
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
}

#[cfg(unix)]
impl std::os::unix::fs::DirBuilderExt for DirBuilder {
    fn mode(&mut self, mode: u32) -> &mut Self {
        self.inner.mode(mode);
        self
    }
}
