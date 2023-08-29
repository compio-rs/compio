use crate::driver::{Entry, Poller};
use std::{
    io,
    os::windows::{
        io::HandleOrNull,
        prelude::{AsRawHandle, OwnedHandle, RawHandle},
    },
    ptr::null_mut,
    task::Poll,
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::{GetLastError, ERROR_HANDLE_EOF, INVALID_HANDLE_VALUE, WAIT_TIMEOUT},
    System::{
        Threading::INFINITE,
        IO::{CreateIoCompletionPort, GetQueuedCompletionStatus, OVERLAPPED},
    },
};

pub(crate) mod fs;
mod op;

pub type RawFd = RawHandle;

pub trait AsRawFd {
    fn as_raw_fd(&self) -> RawFd;
}

pub trait FromRawFd {
    unsafe fn from_raw_fd(fd: RawFd) -> Self;
}

pub trait IntoRawFd {
    fn into_raw_fd(self) -> RawFd;
}

pub trait OpCode {
    unsafe fn operate(&mut self, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>>;
}

impl<T: OpCode + ?Sized> OpCode for &mut T {
    unsafe fn operate(&mut self, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        (**self).operate(optr)
    }
}

pub struct Driver {
    port: OwnedHandle,
}

impl Driver {
    pub fn new() -> io::Result<Self> {
        let port = unsafe { CreateIoCompletionPort(INVALID_HANDLE_VALUE, 0, 0, 0) };
        let port = OwnedHandle::try_from(unsafe { HandleOrNull::from_raw_handle(port as _) })
            .map_err(|_| io::Error::last_os_error())?;
        Ok(Self { port })
    }
}

impl Poller for Driver {
    fn attach(&self, fd: RawFd) -> io::Result<()> {
        let port = unsafe { CreateIoCompletionPort(fd as _, self.port.as_raw_handle() as _, 0, 0) };
        if port == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    fn submit(&self, mut op: impl OpCode, user_data: usize) -> Poll<io::Result<usize>> {
        let overlapped = Box::new(Overlapped::new(user_data));
        let optr = Box::leak(overlapped);
        let res = unsafe { op.operate(optr as *mut Overlapped as *mut OVERLAPPED) };
        if res.is_ready() {
            let _ = unsafe { Box::from_raw(optr) };
        }
        res
    }

    fn poll(&self, timeout: Option<Duration>) -> io::Result<Entry> {
        let mut transferred = 0;
        let mut key = 0;
        let mut overlapped_ptr = null_mut();
        let timeout = match timeout {
            Some(timeout) => timeout.as_millis() as u32,
            None => INFINITE,
        };
        let res = unsafe {
            GetQueuedCompletionStatus(
                self.port.as_raw_handle() as _,
                &mut transferred,
                &mut key,
                &mut overlapped_ptr,
                timeout,
            )
        };
        let result = if res == 0 {
            let error = unsafe { GetLastError() };
            if overlapped_ptr.is_null() {
                return Err(io::Error::from_raw_os_error(error as _));
            }
            match error {
                WAIT_TIMEOUT | ERROR_HANDLE_EOF => Ok(0),
                _ => Err(io::Error::from_raw_os_error(error as _)),
            }
        } else {
            Ok(transferred as usize)
        };
        let overlapped = unsafe { Box::from_raw(overlapped_ptr.cast::<Overlapped>()) };
        Ok(Entry {
            result,
            user_data: overlapped.user_data,
        })
    }
}

#[repr(C)]
struct Overlapped {
    #[allow(dead_code)]
    pub base: OVERLAPPED,
    pub user_data: usize,
}

impl Overlapped {
    pub fn new(user_data: usize) -> Self {
        Self {
            base: unsafe { std::mem::zeroed() },
            user_data,
        }
    }
}
