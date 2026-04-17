use std::{io, os::fd::AsFd, path::Path};

#[cfg(dirfd)]
use compio_driver::ToSharedFd;
use compio_driver::op::{CreateDir, CurrentDir, HardLink, Mode, Rename, Symlink, Unlink};

#[cfg(dirfd)]
use crate::File;
use crate::path_string;

async fn unlink(path: impl AsRef<Path>, dir: bool) -> io::Result<()> {
    let path = path_string(path)?;
    let op = Unlink::new(CurrentDir, path, dir);
    compio_runtime::submit(op).await.0?;
    Ok(())
}

pub async fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
    unlink(path, false).await
}

pub async fn remove_dir(path: impl AsRef<Path>) -> io::Result<()> {
    unlink(path, true).await
}

pub async fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
    let from = path_string(from)?;
    let to = path_string(to)?;
    let op = Rename::new(CurrentDir, from, CurrentDir, to);
    compio_runtime::submit(op).await.0?;
    Ok(())
}

pub async fn symlink(original: impl AsRef<Path>, link: impl AsRef<Path>) -> io::Result<()> {
    let original = path_string(original)?;
    let link = path_string(link)?;
    let op = Symlink::new(original, CurrentDir, link);
    compio_runtime::submit(op).await.0?;
    Ok(())
}

pub async fn hard_link(original: impl AsRef<Path>, link: impl AsRef<Path>) -> io::Result<()> {
    let original = path_string(original)?;
    let link = path_string(link)?;
    let op = HardLink::new(CurrentDir, original, CurrentDir, link);
    compio_runtime::submit(op).await.0?;
    Ok(())
}

pub struct DirBuilder {
    mode: Mode,
}

impl DirBuilder {
    pub fn new() -> Self {
        Self {
            mode: Mode::from_bits_retain(0o777),
        }
    }

    pub fn mode(&mut self, mode: u32) {
        self.mode = Mode::from_bits_retain(mode as _);
    }

    async fn create_impl(&self, dir: impl AsFd + 'static, path: &Path) -> io::Result<()> {
        let path = path_string(path)?;
        let op = CreateDir::new(dir, path, self.mode as _);
        compio_runtime::submit(op).await.0?;
        Ok(())
    }

    pub async fn create(&self, path: &Path) -> io::Result<()> {
        self.create_impl(CurrentDir, path).await
    }

    #[cfg(dirfd)]
    pub async fn create_at(&self, dir: &File, path: &Path) -> io::Result<()> {
        self.create_impl(dir.to_shared_fd(), path).await
    }
}
