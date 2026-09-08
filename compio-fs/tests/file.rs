use std::io::prelude::*;

use compio_fs::{File, OpenOptions};
use compio_io::{AsyncReadAtExt, AsyncReadExt, AsyncWriteAt, AsyncWriteAtExt};
use futures_util::StreamExt;
use tempfile::NamedTempFile;

async fn setlen_run(file: &File, size: u64) {
    file.set_len(size).await.unwrap();
    // For predictability. Give the uring just enough time to ensure that it's
    // completed
    compio_runtime::time::sleep(std::time::Duration::from_millis(0)).await;

    let meta = file.metadata().await.unwrap();
    assert_eq!(size, meta.len());
}

#[compio_macros::test]
async fn iouring_setlen_non_fixed() {
    let tempfile = tempfile();
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(tempfile.path())
        .await
        .unwrap();
    setlen_run(&file, 5).await;

    setlen_run(&file, 0).await;
}

#[compio_macros::test]
async fn metadata() {
    let meta = compio_fs::metadata("Cargo.toml").await.unwrap();
    assert!(meta.is_file());
    let size = meta.len();

    let file = File::open("Cargo.toml").await.unwrap();
    let meta = file.metadata().await.unwrap();
    assert!(meta.is_file());
    assert_eq!(size, meta.len());

    let std_meta = std::fs::metadata("Cargo.toml").unwrap();
    assert_eq!(size, std_meta.len());

    // `created()` must report the birth time and `ctime()` the inode change
    // time, matching std's semantics.
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        assert_eq!(std_meta.ctime(), meta.ctime());
        assert_eq!(std_meta.ctime_nsec(), meta.ctime_nsec());

        if let Ok(std_created) = std_meta.created() {
            assert_eq!(std_created, meta.created().unwrap());
        }
    }
}

#[compio_macros::test]
async fn file_stream() {
    let mut tempfile = tempfile();
    tempfile.write_all(HELLO).unwrap();

    let file = File::open(tempfile.path()).await.unwrap();
    let cursor = std::io::Cursor::new(file);
    let mut file = cursor.read_only().bytes();
    let mut s = String::new();
    while let Some(result) = file.next().await {
        let chunk = result.unwrap();
        s.push_str(str::from_utf8(&chunk).unwrap());
    }
    assert_eq!(s, String::from_utf8_lossy(HELLO));
}

const HELLO: &[u8] = b"hello world...";

async fn read_hello(file: &File) {
    let buf = Vec::with_capacity(1024);
    let (n, buf) = file.read_to_end_at(buf, 0).await.unwrap();

    assert_eq!(n, HELLO.len());
    assert_eq!(&buf, HELLO);
}

#[compio_macros::test]
async fn basic_read() {
    let mut tempfile = tempfile();
    tempfile.write_all(HELLO).unwrap();

    let file = File::open(tempfile.path()).await.unwrap();
    read_hello(&file).await;
}

#[compio_macros::test]
async fn basic_write() {
    let tempfile = tempfile();

    let mut file = File::create(tempfile.path()).await.unwrap();

    file.write_all_at(HELLO, 0).await.0.unwrap();
    file.sync_all().await.unwrap();

    let file = std::fs::read(tempfile.path()).unwrap();
    assert_eq!(file, HELLO);
}

#[compio_macros::test]
async fn writev() {
    let tempfile = tempfile();

    let mut file = File::create(tempfile.path()).await.unwrap();

    let (write, _) = file.write_vectored_at([HELLO, HELLO], 0).await.unwrap();
    assert!(write > 0);
}

#[compio_macros::test]
async fn cancel_read() {
    let mut tempfile = tempfile();
    tempfile.write_all(HELLO).unwrap();

    let file = File::open(tempfile.path()).await.unwrap();

    // Poll the future once, then cancel it
    poll_once(async { read_hello(&file).await }).await;

    read_hello(&file).await;
}

#[cfg(unix)]
#[compio_macros::test]
async fn timeout_read() {
    use std::time::Duration;

    use compio_fs::pipe::anonymous;
    use compio_io::AsyncReadExt;
    use compio_runtime::time::timeout;

    let (mut rx, _) = anonymous().await.unwrap();

    // Read a file with timeout.
    let _ = timeout(Duration::from_nanos(1), async move {
        rx.read_to_string(String::new()).await
    })
    .await
    .unwrap_err();
}

#[compio_macros::test]
async fn drop_open() {
    let tempfile = tempfile();
    let _ = File::create(tempfile.path()).await;

    // Do something else
    let mut file = File::create(tempfile.path()).await.unwrap();

    file.write_all_at(HELLO, 0).await.0.unwrap();

    let file = std::fs::read(tempfile.path()).unwrap();
    assert_eq!(file, HELLO);
}

#[cfg(windows)]
#[compio_macros::test]
async fn hidden_file_truncation() {
    let tmpdir = tempfile::tempdir().unwrap();
    let path = tmpdir.path().join("hidden_file.txt");

    // Create a hidden file.
    const FILE_ATTRIBUTE_HIDDEN: u32 = 2;
    let mut file = compio_fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .attributes(FILE_ATTRIBUTE_HIDDEN)
        .open(&path)
        .await
        .unwrap();
    file.write_all_at("hidden world!", 0).await.unwrap();
    file.close().await.unwrap();

    // Create a new file by truncating the existing one.
    let file = File::create(&path).await.unwrap();
    let metadata = file.metadata().await.unwrap();
    assert_eq!(metadata.len(), 0);
}

fn tempfile() -> NamedTempFile {
    NamedTempFile::new().unwrap()
}

async fn poll_once(future: impl std::future::Future) {
    use std::{future::poll_fn, pin::pin, task::Poll};

    let mut future = pin!(future);

    poll_fn(|cx| {
        let _ = future.as_mut().poll(cx);
        Poll::Ready(())
    })
    .await;
}

#[cfg(any(target_os = "linux", target_os = "android"))]
mod xattr {
    use std::{ffi::CStr, os::fd::AsRawFd};

    use compio_buf::BufResult;

    use super::*;

    const NAME: &str = "user.compio-test";
    const VALUE: &[u8] = b"attribute\0value\xff";

    fn fixture(value: &[u8]) -> NamedTempFile {
        let file = tempfile();
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
        let BufResult(result, buffer) =
            compio_fs::get_xattr(&link, NAME, Vec::with_capacity(64)).await;
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
}
