//! Filesystem utilities.

#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]
#![cfg_attr(feature = "read_buf", feature(read_buf, core_io_borrowed_buf))]
#![cfg_attr(
    all(windows, feature = "windows_by_handle"),
    feature(windows_by_handle)
)]

mod file;
pub use file::*;

mod open_options;
pub use open_options::*;

mod metadata;
pub use metadata::*;

mod stdio;
pub use stdio::*;

mod utils;
pub use utils::*;

mod async_fd;
pub use async_fd::*;

#[cfg(windows)]
pub mod named_pipe;

#[cfg(unix)]
pub mod pipe;

#[cfg(linux_all)]
mod splice;

#[cfg(unix)]
pub(crate) fn path_string(path: impl AsRef<std::path::Path>) -> std::io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;

    std::ffi::CString::new(path.as_ref().as_os_str().as_bytes().to_vec()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "file name contained an unexpected NUL byte",
        )
    })
}
