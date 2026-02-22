use std::{io, os::fd::AsFd, path::Path};

use compio_driver::{
    ToSharedFd,
    op::{CreateDir, CurrentDir, HardLink, Rename, Symlink, Unlink},
};

use crate::{File, path_string};

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
    mode: u32,
}

impl DirBuilder {
    pub fn new() -> Self {
        Self { mode: 0o777 }
    }

    pub fn mode(&mut self, mode: u32) {
        self.mode = mode;
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

    pub async fn create_at(&self, dir: &File, path: &Path) -> io::Result<()> {
        self.create_impl(dir.to_shared_fd(), path).await
    }
}
