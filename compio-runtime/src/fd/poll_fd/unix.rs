use std::{
    cell::RefCell,
    fmt::Debug,
    io,
    ops::Deref,
    pin::Pin,
    task::{Context, Poll},
};

use compio_buf::{BufResult, IntoInner};
use compio_driver::{
    AsFd, AsRawFd, BorrowedFd, RawFd, SharedFd, ToSharedFd,
    op::{Interest, PollOnce},
};

use crate::Submit;

pub struct PollFd<T: AsFd> {
    inner: SharedFd<T>,
    read_submit: RefCell<Option<Submit<PollOnce<SharedFd<T>>>>>,
    write_submit: RefCell<Option<Submit<PollOnce<SharedFd<T>>>>>,
}

impl<T: AsFd + Debug> Debug for PollFd<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PollFd")
            .field("inner", &self.inner)
            .finish()
    }
}

impl<T: AsFd> PollFd<T> {
    pub fn new(inner: SharedFd<T>) -> io::Result<Self> {
        Ok(Self {
            inner,
            read_submit: RefCell::new(None),
            write_submit: RefCell::new(None),
        })
    }
}

impl<T: AsFd + 'static> PollFd<T> {
    pub async fn accept_ready(&self) -> io::Result<()> {
        self.read_ready().await
    }

    pub async fn connect_ready(&self) -> io::Result<()> {
        self.write_ready().await
    }

    pub async fn read_ready(&self) -> io::Result<()> {
        let op = PollOnce::new(self.to_shared_fd(), Interest::Readable);
        let BufResult(res, _) = crate::submit(op).await;
        res?;
        Ok(())
    }

    pub async fn write_ready(&self) -> io::Result<()> {
        let op = PollOnce::new(self.to_shared_fd(), Interest::Writable);
        let BufResult(res, _) = crate::submit(op).await;
        res?;
        Ok(())
    }

    pub fn poll_read_ready(&self, cx: &mut Context) -> Poll<io::Result<()>> {
        let mut read_submit = self.read_submit.borrow_mut();
        loop {
            match read_submit.as_mut() {
                None => {
                    let op = PollOnce::new(self.to_shared_fd(), Interest::Readable);
                    *read_submit = Some(crate::submit(op));
                }
                Some(f) => match Pin::new(f).poll(cx) {
                    Poll::Ready(BufResult(res, _)) => {
                        *read_submit = None;
                        break Poll::Ready(res.map(|_| ()));
                    }
                    Poll::Pending => break Poll::Pending,
                },
            }
        }
    }

    // No use case for waiting read readiness together with accept readiness.
    pub fn poll_accept_ready(&self, cx: &mut Context) -> Poll<io::Result<()>> {
        self.poll_read_ready(cx)
    }

    pub fn poll_write_ready(&self, cx: &mut Context) -> Poll<io::Result<()>> {
        let mut write_submit = self.write_submit.borrow_mut();
        loop {
            match write_submit.as_mut() {
                None => {
                    let op = PollOnce::new(self.to_shared_fd(), Interest::Writable);
                    *write_submit = Some(crate::submit(op));
                }
                Some(f) => match Pin::new(f).poll(cx) {
                    Poll::Ready(BufResult(res, _)) => {
                        *write_submit = None;
                        break Poll::Ready(res.map(|_| ()));
                    }
                    Poll::Pending => break Poll::Pending,
                },
            }
        }
    }

    // No use case for waiting write readiness together with connect readiness.
    pub fn poll_connect_ready(&self, cx: &mut Context) -> Poll<io::Result<()>> {
        self.poll_write_ready(cx)
    }
}

impl<T: AsFd> IntoInner for PollFd<T> {
    type Inner = SharedFd<T>;

    fn into_inner(self) -> Self::Inner {
        self.inner
    }
}

impl<T: AsFd> ToSharedFd<T> for PollFd<T> {
    fn to_shared_fd(&self) -> SharedFd<T> {
        self.inner.clone()
    }
}

impl<T: AsFd> AsFd for PollFd<T> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.inner.as_fd()
    }
}

impl<T: AsFd> AsRawFd for PollFd<T> {
    fn as_raw_fd(&self) -> RawFd {
        self.inner.as_fd().as_raw_fd()
    }
}

impl<T: AsFd> Deref for PollFd<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
