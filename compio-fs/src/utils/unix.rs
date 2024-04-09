use std::{io, path::Path};

use compio_driver::op::{CreateDir, HardLink, Rename, Symlink, Unlink};
use compio_runtime::Runtime;

use crate::path_string;

async fn unlink(path: impl AsRef<Path>, dir: bool) -> io::Result<()> {
    let path = path_string(path)?;
    let op = Unlink::new(path, dir);
    Runtime::current().submit(op).await.0?;
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
    let op = Rename::new(from, to);
    Runtime::current().submit(op).await.0?;
    Ok(())
}

pub async fn symlink(original: impl AsRef<Path>, link: impl AsRef<Path>) -> io::Result<()> {
    let original = path_string(original)?;
    let link = path_string(link)?;
    let op = Symlink::new(original, link);
    Runtime::current().submit(op).await.0?;
    Ok(())
}

pub async fn hard_link(original: impl AsRef<Path>, link: impl AsRef<Path>) -> io::Result<()> {
    let original = path_string(original)?;
    let link = path_string(link)?;
    let op = HardLink::new(original, link);
    Runtime::current().submit(op).await.0?;
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

    pub async fn create(&self, path: &Path) -> io::Result<()> {
        let path = path_string(path)?;
        let op = CreateDir::new(path, self.mode as _);
        Runtime::current().submit(op).await.0?;
        Ok(())
    }
}
