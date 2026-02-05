use std::os::windows::io::{
    AsRawHandle, AsRawSocket, FromRawHandle, FromRawSocket, RawHandle, RawSocket,
};

use compio_driver::AsFd;

use super::AsyncFd;

impl<T: AsFd + AsRawHandle> AsRawHandle for AsyncFd<T> {
    fn as_raw_handle(&self) -> RawHandle {
        self.inner.as_raw_handle()
    }
}

impl<T: AsFd + AsRawSocket> AsRawSocket for AsyncFd<T> {
    fn as_raw_socket(&self) -> RawSocket {
        self.inner.as_raw_socket()
    }
}

impl<T: AsFd + FromRawHandle> FromRawHandle for AsyncFd<T> {
    unsafe fn from_raw_handle(handle: RawHandle) -> Self {
        unsafe { Self::new_unchecked(FromRawHandle::from_raw_handle(handle)) }
    }
}

impl<T: AsFd + FromRawSocket> FromRawSocket for AsyncFd<T> {
    unsafe fn from_raw_socket(sock: RawSocket) -> Self {
        unsafe { Self::new_unchecked(FromRawSocket::from_raw_socket(sock)) }
    }
}
