//! Linux-specific splice operations.

use std::{
    future::{Future, IntoFuture},
    io::{self},
    os::fd::AsFd,
    pin::Pin,
    task::{Context, Poll, ready},
};

use compio_driver::{SharedFd, ToSharedFd, op::Splice as SpliceOp};
use compio_runtime::Submit;

/// Splice data between two file descriptors without copying through userspace.
///
/// At least one of `fd_in` or `fd_out` must be a pipe. Returns a builder that
/// can be configured with optional offsets and flags before awaiting.
///
/// # Example
///
/// ```ignore
/// const HELLO: &[u8] = b"hello world";
///
/// let mut tempfile = tempfile();
/// tempfile.write_all(HELLO).unwrap();
///
/// let file = File::open(tempfile.path()).await.unwrap();
/// let (mut rx, tx) = anonymous().unwrap();
///
/// let n = splice(&file, &tx, HELLO.len()).offset_in(0).await.unwrap();
/// assert_eq!(n, HELLO.len());
///
/// drop(tx);
/// let (_, buf) = rx
///    .read_exact(Vec::with_capacity(HELLO.len()))
///    .await
///    .unwrap();
/// assert_eq!(&buf, HELLO);
/// ```
pub fn splice<I: AsFd + 'static, O: AsFd + 'static>(
    fd_in: &impl ToSharedFd<I>,
    fd_out: &impl ToSharedFd<O>,
    len: usize,
) -> Splice<I, O> {
    Splice {
        fd_in: fd_in.to_shared_fd(),
        fd_out: fd_out.to_shared_fd(),
        len,
        offset_in: -1,
        offset_out: -1,
        flags: 0,
    }
}

/// Builder for splice operations.
pub struct Splice<I, O> {
    fd_in: SharedFd<I>,
    fd_out: SharedFd<O>,
    len: usize,
    offset_in: i64,
    offset_out: i64,
    flags: u32,
}

impl<I, O> Splice<I, O> {
    /// Set the offset to read from.
    pub fn offset_in(mut self, offset: i64) -> Self {
        self.offset_in = offset;
        self
    }

    /// Set the offset to write to.
    pub fn offset_out(mut self, offset: i64) -> Self {
        self.offset_out = offset;
        self
    }

    /// Set splice flags.
    pub fn flags(mut self, flags: u32) -> Self {
        self.flags = flags;
        self
    }
}

impl<I: AsFd + 'static, O: AsFd + 'static> IntoFuture for Splice<I, O> {
    type IntoFuture = SpliceFuture<I, O>;
    type Output = io::Result<usize>;

    fn into_future(self) -> Self::IntoFuture {
        let inner = compio_runtime::submit(SpliceOp::new(
            self.fd_in,
            self.offset_in,
            self.fd_out,
            self.offset_out,
            self.len,
            self.flags,
        ));
        SpliceFuture { inner }
    }
}

pin_project_lite::pin_project! {
    /// Future for splice operations.
    pub struct SpliceFuture<I, O> where I : AsFd, O: AsFd {
        #[pin]
        inner: Submit<SpliceOp<SharedFd<I>, SharedFd<O>>>,
    }
}

impl<I: AsFd + 'static, O: AsFd + 'static> Future for SpliceFuture<I, O> {
    type Output = io::Result<usize>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let res = ready!(self.project().inner.poll(cx));
        Poll::Ready(res.0)
    }
}
