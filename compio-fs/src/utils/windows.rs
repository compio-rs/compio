use std::{io, path::Path};

#[cfg(dirfd)]
use compio_driver::ToSharedFd;

#[cfg(dirfd)]
use crate::File;
use crate::spawn_blocking_named;

pub async fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
    let path = path.as_ref().to_path_buf();
    spawn_blocking_named("fs::remove_file", move || std::fs::remove_file(path)).await
}

pub async fn remove_dir(path: impl AsRef<Path>) -> io::Result<()> {
    let path = path.as_ref().to_path_buf();
    spawn_blocking_named("fs::remove_dir", move || std::fs::remove_dir(path)).await
}

pub async fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
    let from = from.as_ref().to_path_buf();
    let to = to.as_ref().to_path_buf();
    spawn_blocking_named("fs::rename", move || std::fs::rename(from, to)).await
}

pub async fn symlink_file(original: impl AsRef<Path>, link: impl AsRef<Path>) -> io::Result<()> {
    let original = original.as_ref().to_path_buf();
    let link = link.as_ref().to_path_buf();
    spawn_blocking_named("fs::symlink_file", move || {
        std::os::windows::fs::symlink_file(original, link)
    })
    .await
}

pub async fn symlink_dir(original: impl AsRef<Path>, link: impl AsRef<Path>) -> io::Result<()> {
    let original = original.as_ref().to_path_buf();
    let link = link.as_ref().to_path_buf();
    spawn_blocking_named("fs::symlink_dir", move || {
        std::os::windows::fs::symlink_dir(original, link)
    })
    .await
}

pub async fn hard_link(original: impl AsRef<Path>, link: impl AsRef<Path>) -> io::Result<()> {
    let original = original.as_ref().to_path_buf();
    let link = link.as_ref().to_path_buf();
    spawn_blocking_named("fs::hard_link", move || std::fs::hard_link(original, link)).await
}

pub struct DirBuilder;

impl DirBuilder {
    pub fn new() -> Self {
        Self
    }

    pub async fn create(&self, path: &Path) -> io::Result<()> {
        let path = path.to_path_buf();
        spawn_blocking_named("fs::create_dir", move || std::fs::create_dir(path)).await
    }

    #[cfg(dirfd)]
    pub async fn create_at(&self, dir: &File, path: &Path) -> io::Result<()> {
        let path = path.to_path_buf();
        crate::spawn_blocking_with(dir.to_shared_fd(), move |dir| {
            cap_primitives::fs::create_dir(dir, &path, &cap_primitives::fs::DirOptions::new())
        })
        .await
    }
}
