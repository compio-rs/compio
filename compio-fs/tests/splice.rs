#![cfg(linux_all)]

use std::{
    ops::{Deref, DerefMut},
    rc::Rc,
};

use compio_fs::{
    File,
    pipe::{anonymous, splice},
};
use compio_io::{AsyncRead, AsyncReadExt, AsyncWriteAt, AsyncWriteExt};
use compio_net::UnixStream;
use compio_runtime::Runtime;
use futures_util::future::join;
use tempfile::{NamedTempFile, TempPath};

const HELLO: &[u8] = b"hello world...";

struct Guard {
    stream: UnixStream,
    _inner: Rc<TempPath>,
}

impl Deref for Guard {
    type Target = UnixStream;

    fn deref(&self) -> &Self::Target {
        &self.stream
    }
}

impl DerefMut for Guard {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.stream
    }
}

async fn uds() -> (Guard, Guard) {
    let path: Rc<_> = tempfile::Builder::new()
        .prefix("compio-")
        .suffix(".sock")
        .tempfile()
        .expect("failed to create random path for domain socket")
        .into_temp_path()
        .into();

    _ = compio_fs::remove_file(&*path).await;

    let listener = compio_net::UnixListener::bind(&*path).await.unwrap();
    let (a, b) = join(UnixStream::connect(&*path), listener.accept()).await;
    (
        Guard {
            stream: a.unwrap(),
            _inner: path.clone(),
        },
        Guard {
            stream: b.unwrap().0,
            _inner: path,
        },
    )
}

#[compio_macros::test]
async fn splice_uds_to_pipe() {
    let (r, mut w) = uds().await;
    w.write_all(HELLO).await.unwrap();

    let (mut rx, tx) = anonymous().unwrap();

    let n = splice(&*r, &tx, HELLO.len()).await.unwrap();
    assert_eq!(n, HELLO.len());

    drop(tx);
    let (_, buf) = rx
        .read_exact(Vec::with_capacity(HELLO.len()))
        .await
        .unwrap();
    assert_eq!(&buf, HELLO);
}

#[compio_macros::test]
async fn splice_pipe_to_uds() {
    let (mut r, w) = uds().await;
    let (rx, mut tx) = anonymous().unwrap();

    tx.write_all(HELLO).await.unwrap();
    drop(tx);

    let n = splice(&rx, &*w, HELLO.len()).await.unwrap();
    assert_eq!(n, HELLO.len());

    let (len, contents) = r.read(Vec::with_capacity(HELLO.len() + 10)).await.unwrap();
    assert_eq!(len, HELLO.len());
    assert_eq!(&contents, HELLO);
}

#[compio_macros::test]
async fn splice_file_to_pipe() {
    // This test only works with io_uring because splice with offset on files
    // requires io_uring support
    let driver_type = Runtime::with_current(|rt| rt.driver_type());
    if !driver_type.is_iouring() {
        return;
    }

    // Create a temporary file that will be automatically cleaned up
    let temp_file = NamedTempFile::new().unwrap();
    let temp_path = temp_file.path();

    // Create and write to the temporary file
    {
        let mut file = File::create(temp_path).await.unwrap();
        file.write_at(HELLO, 0).await.unwrap();
        file.sync_all().await.unwrap();
    }

    // Open file for reading and splice to pipe
    let file = File::open(temp_path).await.unwrap();
    let (mut rx, tx) = anonymous().unwrap();

    let n = splice(&file, &tx, HELLO.len()).offset_in(0).await.unwrap();
    assert_eq!(n, HELLO.len());

    drop(tx);
    let (_, buf) = rx
        .read_exact(Vec::with_capacity(HELLO.len()))
        .await
        .unwrap();
    assert_eq!(&buf, HELLO);

    // temp_file is automatically cleaned up when dropped
}

#[compio_macros::test]
async fn splice_pipe_to_pipe() {
    let (rx1, mut tx1) = anonymous().unwrap();
    let (mut rx2, tx2) = anonymous().unwrap();

    tx1.write_all(HELLO).await.unwrap();
    drop(tx1);

    let n = splice(&rx1, &tx2, HELLO.len()).await.unwrap();
    assert_eq!(n, HELLO.len());

    drop(tx2);
    let (_, buf) = rx2
        .read_exact(Vec::with_capacity(HELLO.len()))
        .await
        .unwrap();
    assert_eq!(&buf, HELLO);
}
