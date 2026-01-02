//! Linux-specific splice operations.

use std::{
    future::{Future, IntoFuture},
    io::{self, ErrorKind},
    os::fd::AsFd,
    pin::Pin,
    task::{Context, Poll},
};

use compio_driver::{
    AsRawFd, ToSharedFd,
    op::{Interest, PollOnce, Splice},
    syscall,
};

/// Splice data between two file descriptors without copying through userspace.
///
/// At least one of `fd_in` or `fd_out` must be a pipe. Returns a builder that
/// can be configured with optional offsets and flags before awaiting.
///
/// # Example
/// ```ignore
/// let n = splice(&file, &pipe_tx, 1024).await?;
///
/// let n = splice(&file, &pipe_tx, 1024)
///     .offset_in(0)
///     .flags(libc::SPLICE_F_MOVE)
///     .await?;
/// ```
pub fn splice<TIn: AsFd + 'static, TOut: AsFd + 'static>(
    fd_in: &impl ToSharedFd<TIn>,
    fd_out: &impl ToSharedFd<TOut>,
    len: usize,
) -> SpliceBuilder<TIn, TOut> {
    SpliceBuilder {
        fd_in: fd_in.to_shared_fd(),
        fd_out: fd_out.to_shared_fd(),
        len,
        offset_in: -1,
        offset_out: -1,
        flags: 0,
    }
}

/// Builder for splice operations.
pub struct SpliceBuilder<TIn, TOut> {
    fd_in: compio_driver::SharedFd<TIn>,
    fd_out: compio_driver::SharedFd<TOut>,
    len: usize,
    offset_in: i64,
    offset_out: i64,
    flags: u32,
}

impl<TIn, TOut> SpliceBuilder<TIn, TOut> {
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

/// Future for splice operations.
pub struct SpliceFuture<TIn: AsFd + 'static, TOut: AsFd + 'static> {
    state: SpliceState<TIn, TOut>,
}

enum SpliceState<TIn: AsFd + 'static, TOut: AsFd + 'static> {
    Polling(PollingSplice<TIn, TOut>),
    IoUring(
        compio_runtime::Submit<Splice<compio_driver::SharedFd<TIn>, compio_driver::SharedFd<TOut>>>,
    ),
}

struct PollingSplice<TIn: AsFd + 'static, TOut: AsFd + 'static> {
    fd_in: compio_driver::SharedFd<TIn>,
    fd_out: compio_driver::SharedFd<TOut>,
    offset_in: i64,
    offset_out: i64,
    len: usize,
    flags: u32,
    poll_in: Option<compio_runtime::Submit<PollOnce<compio_driver::SharedFd<TIn>>>>,
    poll_out: Option<compio_runtime::Submit<PollOnce<compio_driver::SharedFd<TOut>>>>,
}

impl<TIn: AsFd + 'static, TOut: AsFd + 'static> PollingSplice<TIn, TOut> {
    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<usize>> {
        loop {
            if self.poll_in.is_none() {
                self.poll_in = Some(compio_runtime::submit(PollOnce::new(
                    self.fd_in.clone(),
                    Interest::Readable,
                )));
            }
            if self.poll_out.is_none() {
                self.poll_out = Some(compio_runtime::submit(PollOnce::new(
                    self.fd_out.clone(),
                    Interest::Writable,
                )));
            }

            // Ignore EPERM. Regular files don't support polling but are always ready.
            if let Poll::Ready(res) = Pin::new(self.poll_in.as_mut().unwrap()).poll(cx) {
                if let Err(e) = &res.0
                    && e.raw_os_error() != Some(libc::EPERM)
                {
                    return Poll::Ready(Err(res.0.unwrap_err()));
                }
                self.poll_in = None;
            }
            if let Poll::Ready(res) = Pin::new(self.poll_out.as_mut().unwrap()).poll(cx) {
                if let Err(e) = &res.0
                    && e.raw_os_error() != Some(libc::EPERM)
                {
                    return Poll::Ready(Err(res.0.unwrap_err()));
                }
                self.poll_out = None;
            }

            if self.poll_in.is_some() || self.poll_out.is_some() {
                return Poll::Pending;
            }

            let mut offset_in = self.offset_in;
            let mut offset_out = self.offset_out;
            match syscall!(libc::splice(
                self.fd_in.as_fd().as_raw_fd(),
                if offset_in < 0 {
                    std::ptr::null_mut()
                } else {
                    &mut offset_in
                },
                self.fd_out.as_fd().as_raw_fd(),
                if offset_out < 0 {
                    std::ptr::null_mut()
                } else {
                    &mut offset_out
                },
                self.len,
                (self.flags | libc::SPLICE_F_NONBLOCK) as _,
            )) {
                Ok(n) => return Poll::Ready(Ok(n as usize)),
                Err(e) if e.kind() == ErrorKind::WouldBlock => continue,
                Err(e) => return Poll::Ready(Err(e)),
            }
        }
    }
}

impl<TIn: AsFd + 'static, TOut: AsFd + 'static> Future for SpliceFuture<TIn, TOut> {
    type Output = io::Result<usize>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match &mut self.state {
            SpliceState::Polling(inner) => inner.poll(cx),
            SpliceState::IoUring(inner) => Pin::new(inner).poll(cx).map(|res| res.0),
        }
    }
}

impl<TIn: AsFd + 'static, TOut: AsFd + 'static> IntoFuture for SpliceBuilder<TIn, TOut> {
    type IntoFuture = SpliceFuture<TIn, TOut>;
    type Output = io::Result<usize>;

    fn into_future(self) -> Self::IntoFuture {
        let state = if compio_runtime::Runtime::with_current(|r| r.driver_type()).is_polling() {
            SpliceState::Polling(PollingSplice {
                fd_in: self.fd_in,
                fd_out: self.fd_out,
                offset_in: self.offset_in,
                offset_out: self.offset_out,
                len: self.len,
                flags: self.flags,
                poll_in: None,
                poll_out: None,
            })
        } else {
            SpliceState::IoUring(compio_runtime::submit(Splice::new(
                self.fd_in,
                self.offset_in,
                self.fd_out,
                self.offset_out,
                self.len,
                self.flags,
            )))
        };
        SpliceFuture { state }
    }
}
