#![cfg(any(target_os = "linux", target_os = "android"))]

use std::{ffi::CStr, os::fd::AsRawFd};

use compio_buf::BufResult;
use compio_fs::File;
use tempfile::NamedTempFile;

const NAME: &str = "user.compio-test";
const VALUE: &[u8] = b"attribute\0value\xff";

fn fixture(value: &[u8]) -> NamedTempFile {
    let file = NamedTempFile::new().unwrap();
    let name: &CStr = c"user.compio-test";
    // SAFETY: The file owns its live descriptor for the whole call; name is
    // NUL-terminated and value remains readable for its specified length.
    let result = unsafe {
        libc::fsetxattr(
            file.as_raw_fd(),
            name.as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
        )
    };
    assert_eq!(result, 0, "{}", std::io::Error::last_os_error());
    file
}

#[compio_macros::test]
async fn read_value() {
    let tempfile = fixture(VALUE);
    let file = File::open(tempfile.path()).await.unwrap();

    for result in [
        compio_fs::get_xattr(tempfile.path(), NAME, Vec::with_capacity(64)).await,
        file.get_xattr(NAME, Vec::with_capacity(64)).await,
    ] {
        let (length, buffer) = result.unwrap();
        assert_eq!(length, VALUE.len());
        assert_eq!(buffer, VALUE);
    }
}

#[compio_macros::test]
async fn size_query() {
    let tempfile = fixture(VALUE);
    let file = File::open(tempfile.path()).await.unwrap();

    let (length, mut buffer) = compio_fs::get_xattr(tempfile.path(), NAME, Vec::new())
        .await
        .unwrap();
    assert_eq!(length, VALUE.len());
    assert!(buffer.is_empty());
    assert_eq!(buffer.capacity(), 0);
    buffer.reserve_exact(length);
    let (read, buffer) = compio_fs::get_xattr(tempfile.path(), NAME, buffer)
        .await
        .unwrap();
    assert_eq!(read, length);
    assert_eq!(buffer, VALUE);

    let (length, mut buffer) = file.get_xattr(NAME, Vec::new()).await.unwrap();
    assert_eq!(length, VALUE.len());
    assert!(buffer.is_empty());
    assert_eq!(buffer.capacity(), 0);
    buffer.reserve_exact(length);
    let (read, buffer) = file.get_xattr(NAME, buffer).await.unwrap();
    assert_eq!(read, length);
    assert_eq!(buffer, VALUE);
}

#[compio_macros::test]
async fn missing_attribute() {
    let tempfile = fixture(VALUE);
    let file = File::open(tempfile.path()).await.unwrap();

    for BufResult(result, buffer) in [
        compio_fs::get_xattr(tempfile.path(), "user.missing", vec![0x55; 32]).await,
        file.get_xattr("user.missing", vec![0x55; 32]).await,
    ] {
        assert_eq!(result.unwrap_err().raw_os_error(), Some(libc::ENODATA));
        assert_eq!(buffer, vec![0x55; 32]);
    }
}

#[compio_macros::test]
async fn undersized_buffer() {
    let tempfile = fixture(VALUE);
    let file = File::open(tempfile.path()).await.unwrap();

    for BufResult(result, buffer) in [
        compio_fs::get_xattr(tempfile.path(), NAME, vec![0x55]).await,
        file.get_xattr(NAME, vec![0x55]).await,
    ] {
        assert_eq!(result.unwrap_err().raw_os_error(), Some(libc::ERANGE));
        assert_eq!(buffer, [0x55]);
    }
}

#[compio_macros::test]
async fn empty_attribute() {
    let tempfile = fixture(b"");
    let file = File::open(tempfile.path()).await.unwrap();

    for result in [
        compio_fs::get_xattr(tempfile.path(), NAME, Vec::new()).await,
        file.get_xattr(NAME, Vec::new()).await,
        compio_fs::get_xattr(tempfile.path(), NAME, Vec::with_capacity(32)).await,
        file.get_xattr(NAME, Vec::with_capacity(32)).await,
    ] {
        let (length, buffer) = result.unwrap();
        assert_eq!(length, 0);
        assert!(buffer.is_empty());
    }
}

#[compio_macros::test]
async fn invalid_input_returns_buffer() {
    let tempfile = fixture(VALUE);
    let file = File::open(tempfile.path()).await.unwrap();

    for BufResult(result, buffer) in [
        compio_fs::get_xattr(tempfile.path(), "user.invalid\0name", vec![0x55]).await,
        file.get_xattr("user.invalid\0name", vec![0x55]).await,
        compio_fs::get_xattr("invalid\0path", NAME, vec![0x55]).await,
    ] {
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::InvalidInput);
        assert_eq!(buffer, [0x55]);
    }
}

#[compio_macros::test]
async fn path_follows_symlink_and_fd_survives_unlink() {
    let tempfile = fixture(VALUE);
    let directory = tempfile::tempdir().unwrap();
    let link = directory.path().join("link");
    std::os::unix::fs::symlink(tempfile.path(), &link).unwrap();
    let file = File::open(&link).await.unwrap();

    let (length, buffer) = compio_fs::get_xattr(&link, NAME, Vec::with_capacity(64))
        .await
        .unwrap();
    assert_eq!(length, VALUE.len());
    assert_eq!(buffer, VALUE);

    std::fs::remove_file(tempfile.path()).unwrap();
    let (length, buffer) = file.get_xattr(NAME, Vec::with_capacity(64)).await.unwrap();
    assert_eq!(length, VALUE.len());
    assert_eq!(buffer, VALUE);
    let BufResult(result, buffer) = compio_fs::get_xattr(&link, NAME, Vec::with_capacity(64)).await;
    assert_eq!(result.unwrap_err().raw_os_error(), Some(libc::ENOENT));
    assert!(buffer.is_empty());
}

#[compio_macros::test]
async fn cancel_read_keeps_file_usable() {
    let tempfile = fixture(VALUE);
    let file = File::open(tempfile.path()).await.unwrap();

    // Match the existing file-read cancellation test: poll once, then drop.
    // A backend may complete immediately instead of remaining pending.
    poll_once(file.get_xattr(NAME, Vec::with_capacity(64))).await;

    let (length, buffer) = file.get_xattr(NAME, Vec::with_capacity(64)).await.unwrap();
    assert_eq!(length, VALUE.len());
    assert_eq!(buffer, VALUE);
}

async fn poll_once(future: impl Future) {
    use std::{future::poll_fn, pin::pin, task::Poll};

    let mut future = pin!(future);

    poll_fn(|cx| {
        let _ = future.as_mut().poll(cx);
        Poll::Ready(())
    })
    .await;
}
