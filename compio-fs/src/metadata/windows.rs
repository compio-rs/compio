pub use std::fs::{FileType, Metadata, Permissions};
use std::{io, path::Path};

pub async fn metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    let path = path.as_ref().to_path_buf();
    compio_runtime::spawn_blocking(move || std::fs::metadata(path)).await
}

pub async fn symlink_metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    let path = path.as_ref().to_path_buf();
    compio_runtime::spawn_blocking(move || std::fs::symlink_metadata(path)).await
}

pub async fn set_permissions(path: impl AsRef<Path>, perm: Permissions) -> io::Result<()> {
    let path = path.as_ref().to_path_buf();
    compio_runtime::spawn_blocking(move || std::fs::set_permissions(path, perm)).await
}
