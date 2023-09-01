use crate::{
    driver::{AsRawFd, FromRawFd, IntoRawFd, RawFd},
    net::Socket,
};

impl AsRawFd for Socket {
    fn as_raw_fd(&self) -> RawFd {
        self.as_socket2().as_raw_fd()
    }
}

impl FromRawFd for Socket {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self {
            socket: socket2::Socket::from_raw_fd(fd),
        }
    }
}

impl IntoRawFd for Socket {
    fn into_raw_fd(self) -> RawFd {
        self.into_socket2().into_raw_fd()
    }
}
