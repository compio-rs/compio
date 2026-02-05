//! Filesystem utilities.

#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]
#![doc(
    html_logo_url = "https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-bold.svg"
)]
#![doc(
    html_favicon_url = "https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-bold.svg"
)]
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

#[cfg(windows)]
pub mod named_pipe;

#[cfg(unix)]
pub mod pipe;

/// Providing functionalities to wait for readiness.
#[deprecated(since = "0.12.0", note = "Use `compio_runtime::fd::AsyncFd` instead")]
pub type AsyncFd<T> = compio_runtime::fd::AsyncFd<T>;

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
