use std::{io, os::windows::fs::MetadataExt, panic::resume_unwind, path::Path, time::SystemTime};

use compio_driver::ToSharedFd;

use crate::File;

pub async fn metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    let path = path.as_ref().to_path_buf();
    compio_runtime::spawn_blocking(move || std::fs::metadata(path))
        .await
        .unwrap_or_else(|e| resume_unwind(e))
        .map(Metadata::from)
}

pub async fn symlink_metadata(path: impl AsRef<Path>) -> io::Result<Metadata> {
    let path = path.as_ref().to_path_buf();
    compio_runtime::spawn_blocking(move || std::fs::symlink_metadata(path))
        .await
        .unwrap_or_else(|e| resume_unwind(e))
        .map(Metadata::from)
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
    .await
    .map(Metadata::from)
}

pub async fn metadata_at(dir: &File, path: impl AsRef<Path>) -> io::Result<Metadata> {
    metadata_at_impl(dir, path, true).await
}

pub async fn symlink_metadata_at(dir: &File, path: impl AsRef<Path>) -> io::Result<Metadata> {
    metadata_at_impl(dir, path, false).await
}

pub async fn set_permissions(path: impl AsRef<Path>, perm: Permissions) -> io::Result<()> {
    let path = path.as_ref().to_path_buf();
    compio_runtime::spawn_blocking(move || {
        let f = std::fs::File::open(path)?;
        let mut p = f.metadata()?.permissions();
        p.set_readonly(perm.readonly());
        f.set_permissions(p)
    })
    .await
    .unwrap_or_else(|e| resume_unwind(e))
}

#[derive(Clone)]
pub struct Metadata {
    attributes: u32,
    creation_time: u64,
    last_access_time: u64,
    last_write_time: u64,
    file_size: u64,
    volume_serial_number: Option<u32>,
    number_of_links: Option<u32>,
    file_index: Option<u64>,
    file_type: FileType,
    permissions: Permissions,
    modified: SystemTime,
    accessed: SystemTime,
    created: SystemTime,
}

impl Metadata {
    pub fn file_type(&self) -> FileType {
        self.file_type
    }

    pub fn is_dir(&self) -> bool {
        self.file_type.is_dir()
    }

    pub fn is_file(&self) -> bool {
        self.file_type.is_file()
    }

    pub fn is_symlink(&self) -> bool {
        self.file_type.is_symlink()
    }

    pub fn len(&self) -> u64 {
        self.file_size
    }

    pub fn permissions(&self) -> Permissions {
        self.permissions
    }

    pub fn modified(&self) -> io::Result<SystemTime> {
        Ok(self.modified)
    }

    pub fn accessed(&self) -> io::Result<SystemTime> {
        Ok(self.accessed)
    }

    pub fn created(&self) -> io::Result<SystemTime> {
        Ok(self.created)
    }

    pub fn file_attributes(&self) -> u32 {
        self.attributes
    }

    pub fn creation_time(&self) -> u64 {
        self.creation_time
    }

    pub fn last_access_time(&self) -> u64 {
        self.last_access_time
    }

    pub fn last_write_time(&self) -> u64 {
        self.last_write_time
    }

    pub fn volume_serial_number(&self) -> Option<u32> {
        self.volume_serial_number
    }

    pub fn number_of_links(&self) -> Option<u32> {
        self.number_of_links
    }

    pub fn file_index(&self) -> Option<u64> {
        self.file_index
    }
}

impl From<std::fs::Metadata> for Metadata {
    fn from(value: std::fs::Metadata) -> Self {
        Self {
            attributes: value.file_attributes(),
            creation_time: value.creation_time(),
            last_access_time: value.last_access_time(),
            last_write_time: value.last_write_time(),
            file_size: value.file_size(),
            volume_serial_number: value.volume_serial_number(),
            number_of_links: value.number_of_links(),
            file_index: value.file_index(),
            file_type: value.file_type().into(),
            permissions: value.permissions().into(),
            modified: value
                .modified()
                .expect("std::fs::Metadata::modified() should never fail on Windows"),
            accessed: value
                .accessed()
                .expect("std::fs::Metadata::accessed() should never fail on Windows"),
            created: value
                .created()
                .expect("std::fs::Metadata::created() should never fail on Windows"),
        }
    }
}

impl From<cap_primitives::fs::Metadata> for Metadata {
    fn from(value: cap_primitives::fs::Metadata) -> Self {
        use cap_primitives::fs::MetadataExt;

        Self {
            attributes: value.file_attributes(),
            creation_time: value.creation_time(),
            last_access_time: value.last_access_time(),
            last_write_time: value.last_write_time(),
            file_size: value.file_size(),
            volume_serial_number: value.volume_serial_number(),
            number_of_links: value.number_of_links(),
            file_index: value.file_index(),
            file_type: value.file_type().into(),
            permissions: value.permissions().into(),
            modified: value
                .modified()
                .expect("cap_primitives::fs::Metadata::modified() should never fail on Windows")
                .into_std(),
            accessed: value
                .accessed()
                .expect("cap_primitives::fs::Metadata::accessed() should never fail on Windows")
                .into_std(),
            created: value
                .created()
                .expect("cap_primitives::fs::Metadata::created() should never fail on Windows")
                .into_std(),
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Permissions {
    readonly: bool,
}

impl Permissions {
    pub fn readonly(&self) -> bool {
        self.readonly
    }

    pub fn set_readonly(&mut self, readonly: bool) {
        self.readonly = readonly;
    }
}

impl From<std::fs::Permissions> for Permissions {
    fn from(value: std::fs::Permissions) -> Self {
        Self {
            readonly: value.readonly(),
        }
    }
}

impl From<cap_primitives::fs::Permissions> for Permissions {
    fn from(value: cap_primitives::fs::Permissions) -> Self {
        Self {
            readonly: value.readonly(),
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct FileType {
    is_dir: bool,
    is_symlink: bool,
}

impl FileType {
    pub fn is_dir(&self) -> bool {
        self.is_dir && !self.is_symlink
    }

    pub fn is_file(&self) -> bool {
        !self.is_dir && !self.is_symlink
    }

    pub fn is_symlink(&self) -> bool {
        self.is_symlink
    }

    pub fn is_symlink_dir(&self) -> bool {
        self.is_symlink && self.is_dir
    }

    pub fn is_symlink_file(&self) -> bool {
        self.is_symlink && !self.is_dir
    }
}

impl From<std::fs::FileType> for FileType {
    fn from(value: std::fs::FileType) -> Self {
        Self {
            is_dir: value.is_dir(),
            is_symlink: value.is_symlink(),
        }
    }
}

impl From<cap_primitives::fs::FileType> for FileType {
    fn from(value: cap_primitives::fs::FileType) -> Self {
        Self {
            is_dir: value.is_dir(),
            is_symlink: value.is_symlink(),
        }
    }
}
