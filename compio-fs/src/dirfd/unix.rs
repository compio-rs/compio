use std::{io, path::Path};

use compio_driver::{
    ToSharedFd,
    op::{HardLink, Rename, Symlink, Unlink},
};

use crate::{
    DirBuilder, File, Metadata, OpenOptions, metadata_at, path_string, symlink_metadata_at,
};

#[derive(Debug, Clone)]
pub struct Dir {
    inner: File,
}

impl Dir {
    pub async fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY)
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
                    .custom_flags(libc::O_DIRECTORY),
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
        let source = path_string(source)?;
        let target = path_string(target)?;
        let op = HardLink::new(
            self.inner.to_shared_fd(),
            source,
            target_dir.inner.to_shared_fd(),
            target,
        );
        compio_runtime::submit(op).await.0?;
        Ok(())
    }

    pub async fn symlink(
        &self,
        original: impl AsRef<Path>,
        link: impl AsRef<Path>,
    ) -> io::Result<()> {
        let original = path_string(original)?;
        let link = path_string(link)?;
        let op = Symlink::new(original, self.inner.to_shared_fd(), link);
        compio_runtime::submit(op).await.0?;
        Ok(())
    }

    pub async fn rename(
        &self,
        from: impl AsRef<Path>,
        to_dir: &Self,
        to: impl AsRef<Path>,
    ) -> io::Result<()> {
        let from = path_string(from)?;
        let to = path_string(to)?;
        let op = Rename::new(
            self.inner.to_shared_fd(),
            from,
            to_dir.inner.to_shared_fd(),
            to,
        );
        compio_runtime::submit(op).await.0?;
        Ok(())
    }

    pub async fn remove_file(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let path = path_string(path)?;
        let op = Unlink::new(self.inner.to_shared_fd(), path, false);
        compio_runtime::submit(op).await.0?;
        Ok(())
    }

    pub async fn remove_dir(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let path = path_string(path)?;
        let op = Unlink::new(self.inner.to_shared_fd(), path, true);
        compio_runtime::submit(op).await.0?;
        Ok(())
    }
}

compio_driver::impl_raw_fd!(Dir, std::fs::File, inner);
