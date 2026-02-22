use std::{io, panic::resume_unwind, path::Path};

use compio_driver::ToSharedFd;

use crate::File;

pub async fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
    let path = path.as_ref().to_path_buf();
    compio_runtime::spawn_blocking(move || std::fs::remove_file(path))
        .await
        .unwrap_or_else(|e| resume_unwind(e))
}

pub async fn remove_dir(path: impl AsRef<Path>) -> io::Result<()> {
    let path = path.as_ref().to_path_buf();
    compio_runtime::spawn_blocking(move || std::fs::remove_dir(path))
        .await
        .unwrap_or_else(|e| resume_unwind(e))
}

pub async fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
    let from = from.as_ref().to_path_buf();
    let to = to.as_ref().to_path_buf();
    compio_runtime::spawn_blocking(move || std::fs::rename(from, to))
        .await
        .unwrap_or_else(|e| resume_unwind(e))
}

pub async fn symlink_file(original: impl AsRef<Path>, link: impl AsRef<Path>) -> io::Result<()> {
    let original = original.as_ref().to_path_buf();
    let link = link.as_ref().to_path_buf();
    compio_runtime::spawn_blocking(move || std::os::windows::fs::symlink_file(original, link))
        .await
        .unwrap_or_else(|e| resume_unwind(e))
}

pub async fn symlink_dir(original: impl AsRef<Path>, link: impl AsRef<Path>) -> io::Result<()> {
    let original = original.as_ref().to_path_buf();
    let link = link.as_ref().to_path_buf();
    compio_runtime::spawn_blocking(move || std::os::windows::fs::symlink_dir(original, link))
        .await
        .unwrap_or_else(|e| resume_unwind(e))
}

pub async fn hard_link(original: impl AsRef<Path>, link: impl AsRef<Path>) -> io::Result<()> {
    let original = original.as_ref().to_path_buf();
    let link = link.as_ref().to_path_buf();
    compio_runtime::spawn_blocking(move || std::fs::hard_link(original, link))
        .await
        .unwrap_or_else(|e| resume_unwind(e))
}

pub struct DirBuilder;

impl DirBuilder {
    pub fn new() -> Self {
        Self
    }

    pub async fn create(&self, path: &Path) -> io::Result<()> {
        let path = path.to_path_buf();
        compio_runtime::spawn_blocking(move || std::fs::create_dir(path))
            .await
            .unwrap_or_else(|e| resume_unwind(e))
    }

    pub async fn create_at(&self, dir: &File, path: &Path) -> io::Result<()> {
        let path = path.to_path_buf();
        crate::spawn_blocking_with(dir.to_shared_fd(), move |dir| {
            cap_primitives::fs::create_dir(dir, &path, &cap_primitives::fs::DirOptions::new())
        })
        .await
    }
}
