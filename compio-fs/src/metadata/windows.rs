pub use std::fs::{FileType, Metadata, Permissions};
use std::{io, panic::resume_unwind, path::Path};

use compio_driver::ToSharedFd;

use crate::File;

pub async fn metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    let path = path.as_ref().to_path_buf();
    compio_runtime::spawn_blocking(move || std::fs::metadata(path))
        .await
        .unwrap_or_else(|e| resume_unwind(e))
}

pub async fn symlink_metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    let path = path.as_ref().to_path_buf();
    compio_runtime::spawn_blocking(move || std::fs::symlink_metadata(path))
        .await
        .unwrap_or_else(|e| resume_unwind(e))
}

async fn metadata_at_impl(
    dir: &File,
    path: impl AsRef<Path>,
    follow_symlinks: bool,
) -> io::Result<Metadata> {
    let path = path.as_ref().to_path_buf();
    crate::spawn_blocking_with(dir.to_shared_fd(), move |dir| {
        cap_primitives::fs::stat(
            dir,
            &path,
            if follow_symlinks {
                cap_primitives::fs::FollowSymlinks::Yes
            } else {
                cap_primitives::fs::FollowSymlinks::No
            },
        )
    })
    .await?;
    todo!()
}

pub async fn metadata_at(dir: &File, path: impl AsRef<Path>) -> io::Result<Metadata> {
    metadata_at_impl(dir, path, true).await
}

pub async fn symlink_metadata_at(dir: &File, path: impl AsRef<Path>) -> io::Result<Metadata> {
    metadata_at_impl(dir, path, false).await
}

pub async fn set_permissions(path: impl AsRef<Path>, perm: Permissions) -> io::Result<()> {
    let path = path.as_ref().to_path_buf();
    compio_runtime::spawn_blocking(move || std::fs::set_permissions(path, perm))
        .await
        .unwrap_or_else(|e| resume_unwind(e))
}
