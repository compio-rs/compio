use std::{io, path::Path};

use compio_driver::ToSharedFd;
use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS;

use crate::{DirBuilder, File, Metadata, OpenOptions, metadata_at, symlink_metadata_at};

#[derive(Debug, Clone)]
pub struct Dir {
    inner: File,
}

impl Dir {
    pub async fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
            .await?;
        Ok(Dir { inner: file })
    }

    pub async fn open_file_with(
        &self,
        path: impl AsRef<Path>,
        options: &OpenOptions,
    ) -> io::Result<File> {
        options.open_at(&self.inner, path).await
    }

    pub async fn open_dir(&self, path: impl AsRef<Path>) -> io::Result<Self> {
        let file = self
            .open_file_with(
                path,
                OpenOptions::new()
                    .read(true)
                    .custom_flags(FILE_FLAG_BACKUP_SEMANTICS),
            )
            .await?;
        Ok(Dir { inner: file })
    }

    pub async fn create_dir_with(
        &self,
        path: impl AsRef<Path>,
        builder: &DirBuilder,
    ) -> io::Result<()> {
        builder.create_at(&self.inner, path.as_ref()).await
    }

    pub async fn dir_metadata(&self) -> io::Result<Metadata> {
        self.inner.metadata().await
    }

    pub async fn metadata(&self, path: impl AsRef<Path>) -> io::Result<Metadata> {
        metadata_at(&self.inner, path).await
    }

    pub async fn symlink_metadata(&self, path: impl AsRef<Path>) -> io::Result<Metadata> {
        symlink_metadata_at(&self.inner, path).await
    }

    pub async fn hard_link(
        &self,
        source: impl AsRef<Path>,
        target_dir: &Self,
        target: impl AsRef<Path>,
    ) -> io::Result<()> {
        let source = source.as_ref().to_path_buf();
        let target = target.as_ref().to_path_buf();
        crate::spawn_blocking_with2(
            self.to_shared_fd(),
            target_dir.to_shared_fd(),
            move |sdir, tdir| cap_primitives::fs::hard_link(sdir, &source, tdir, &target),
        )
        .await
    }

    pub async fn rename(
        &self,
        from: impl AsRef<Path>,
        to_dir: &Self,
        to: impl AsRef<Path>,
    ) -> io::Result<()> {
        let from = from.as_ref().to_path_buf();
        let to = to.as_ref().to_path_buf();
        crate::spawn_blocking_with2(
            self.to_shared_fd(),
            to_dir.to_shared_fd(),
            move |fdir, tdir| cap_primitives::fs::rename(fdir, &from, tdir, &to),
        )
        .await
    }

    pub async fn remove_file(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let path = path.as_ref().to_path_buf();
        crate::spawn_blocking_with(self.to_shared_fd(), move |dir| {
            cap_primitives::fs::remove_file(dir, &path)
        })
        .await
    }

    pub async fn remove_dir(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let path = path.as_ref().to_path_buf();
        crate::spawn_blocking_with(self.to_shared_fd(), move |dir| {
            cap_primitives::fs::remove_dir(dir, &path)
        })
        .await
    }
}

compio_driver::impl_raw_fd!(Dir, std::fs::File, inner);
