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

#[cfg(dirfd)]
mod dirfd;
#[cfg(dirfd)]
pub use dirfd::*;

#[cfg(windows)]
pub mod named_pipe;

#[cfg(unix)]
pub mod pipe;

/// Providing functionalities to wait for readiness.
#[deprecated(since = "0.12.0", note = "Use `compio::runtime::fd::AsyncFd` instead")]
pub type AsyncFd<T> = compio_runtime::fd::AsyncFd<T>;

use std::io;

#[cfg(unix)]
pub(crate) fn path_string(path: impl AsRef<std::path::Path>) -> io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;

    std::ffi::CString::new(path.as_ref().as_os_str().as_bytes().to_vec()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "file name contained an unexpected NUL byte",
        )
    })
}

use compio_buf::{BufResult, IntoInner};
use compio_driver::{SharedFd, op::AsyncifyFd};

pub(crate) async fn spawn_blocking_with<T: 'static, R: Send + 'static>(
    fd: SharedFd<T>,
    f: impl FnOnce(&T) -> io::Result<R> + Send + 'static,
) -> io::Result<R> {
    let op = AsyncifyFd::new(fd, move |fd: &T| match f(fd) {
        Ok(res) => BufResult(Ok(0), Some(res)),
        Err(e) => BufResult(Err(e), None),
    });
    let BufResult(res, meta) = compio_runtime::submit(op).await;
    res?;
    Ok(meta.into_inner().expect("result should be present"))
}

#[cfg(all(windows, dirfd))]
pub(crate) async fn spawn_blocking_with2<T1: 'static, T2: 'static, R: Send + 'static>(
    fd1: SharedFd<T1>,
    fd2: SharedFd<T2>,
    f: impl FnOnce(&T1, &T2) -> io::Result<R> + Send + 'static,
) -> io::Result<R> {
    use compio_driver::op::AsyncifyFd2;

    let op = AsyncifyFd2::new(fd1, fd2, move |fd1: &T1, fd2: &T2| match f(fd1, fd2) {
        Ok(res) => BufResult(Ok(0), Some(res)),
        Err(e) => BufResult(Err(e), None),
    });
    let BufResult(res, meta) = compio_runtime::submit(op).await;
    res?;
    Ok(meta.into_inner().expect("result should be present"))
}
