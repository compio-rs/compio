use std::{io, ops::Deref};

use compio_buf::{BufResult, IntoInner};
use compio_driver::{
    AsFd, AsRawFd, BorrowedFd, RawFd, SharedFd, ToSharedFd,
    op::{Interest, PollOnce},
};

#[derive(Debug)]
pub struct PollFd<T: AsFd> {
    inner: SharedFd<T>,
}

impl<T: AsFd> PollFd<T> {
    pub fn new(inner: SharedFd<T>) -> io::Result<Self> {
        Ok(Self { inner })
    }
}

impl<T: AsFd + AsRawFd + 'static> PollFd<T> {
    pub async fn accept_ready(&self) -> io::Result<()> {
        self.read_ready().await
    }

    pub async fn connect_ready(&self) -> io::Result<()> {
        self.write_ready().await
    }

    pub async fn read_ready(&self) -> io::Result<()> {
        let op = PollOnce::new(self.to_shared_fd(), Interest::Readable);
        let BufResult(res, _) = compio_runtime::submit(op).await;
        res?;
        Ok(())
    }

    pub async fn write_ready(&self) -> io::Result<()> {
        let op = PollOnce::new(self.to_shared_fd(), Interest::Writable);
        let BufResult(res, _) = compio_runtime::submit(op).await;
        res?;
        Ok(())
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
    fn as_fd(&self) -> BorrowedFd {
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
