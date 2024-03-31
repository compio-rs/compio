#[cfg(unix)]
#[path = "unix.rs"]
mod sys;

#[cfg(windows)]
#[path = "windows.rs"]
mod sys;

use std::{io, path::Path};

/// Removes a file from the filesystem.
pub async fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
    sys::remove_file(path).await
}

/// Removes an empty directory.
pub async fn remove_dir(path: impl AsRef<Path>) -> io::Result<()> {
    sys::remove_dir(path).await
}

/// Creates a new, empty directory at the provided path
pub async fn create_dir(path: impl AsRef<Path>) -> io::Result<()> {
    sys::create_dir(path).await
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

/// Creates a new hard link on the filesystem.
pub async fn hard_link(original: impl AsRef<Path>, link: impl AsRef<Path>) -> io::Result<()> {
    sys::hard_link(original, link).await
}
