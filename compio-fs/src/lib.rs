//! Filesystem utilities.

#![cfg_attr(docsrs, feature(doc_cfg))]
#![allow(unused_features)]
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

use std::{future::Future, io};

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

/// Run `f` on the blocking pool, reporting it to the console under `name`.
///
/// The tasks compio spawns itself are named, since their location points into
/// compio rather than into the code that asked for the work. It still tells
/// which fallback is running, so this is a plain `fn`: `#[track_caller]` is a
/// no-op on an `async fn`, and every fallback would report this line instead
/// of its own.
#[allow(dead_code)] // Only some platforms have blocking fallbacks.
#[track_caller]
pub(crate) fn spawn_blocking_named<T: Send + 'static>(
    name: &'static str,
    f: impl (FnOnce() -> T) + Send + 'static,
) -> impl Future<Output = T> {
    use compio_runtime::{ResumeUnwind, SpawnMeta};

    // Captured before the future, which is where the caller is lost.
    let meta = SpawnMeta::capture().named(name);

    async move {
        compio_runtime::spawn_blocking_at(f, meta)
            .await
            .resume_unwind()
            .expect("shouldn't be cancelled")
    }
}

pub(crate) async fn spawn_blocking_with<T, R, F>(fd: SharedFd<T>, f: F) -> io::Result<R>
where
    T: Sync + 'static,
    R: Send + 'static,
    F: FnOnce(&T) -> io::Result<R> + Send + 'static,
{
    let op = AsyncifyFd::new(fd, move |fd: &T| match f(fd) {
        Ok(res) => BufResult(Ok(0), Some(res)),
        Err(e) => BufResult(Err(e), None),
    });
    let BufResult(res, meta) = compio_runtime::submit(op).await;
    res?;
    Ok(meta.into_inner().expect("result should be present"))
}

#[cfg(all(windows, dirfd))]
pub(crate) async fn spawn_blocking_with2<T1, T2, R, F>(
    fd1: SharedFd<T1>,
    fd2: SharedFd<T2>,
    f: F,
) -> io::Result<R>
where
    T1: Sync + 'static,
    T2: Sync + 'static,
    R: Send + 'static,
    F: FnOnce(&T1, &T2) -> io::Result<R> + Send + 'static,
{
    use compio_driver::op::AsyncifyFd2;

    let op = AsyncifyFd2::new(fd1, fd2, move |fd1: &T1, fd2: &T2| match f(fd1, fd2) {
        Ok(res) => BufResult(Ok(0), Some(res)),
        Err(e) => BufResult(Err(e), None),
    });
    let BufResult(res, meta) = compio_runtime::submit(op).await;
    res?;
    Ok(meta.into_inner().expect("result should be present"))
}
