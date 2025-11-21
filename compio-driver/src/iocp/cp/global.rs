#[cfg(feature = "once_cell_try")]
use std::sync::OnceLock;
use std::{
    io,
    os::windows::io::{AsRawHandle, RawHandle},
    time::Duration,
};

use compio_log::*;
#[cfg(not(feature = "once_cell_try"))]
use once_cell::sync::OnceCell as OnceLock;
use windows_sys::Win32::System::IO::PostQueuedCompletionStatus;

use super::CompletionPort;
use crate::{Entry, Overlapped, RawFd, syscall};

struct GlobalPort {
    port: CompletionPort,
}

impl GlobalPort {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            port: CompletionPort::new()?,
        })
    }

    pub fn attach(&self, fd: RawFd) -> io::Result<()> {
        self.port.attach(fd)
    }
}

impl AsRawHandle for GlobalPort {
    fn as_raw_handle(&self) -> RawHandle {
        self.port.as_raw_handle()
    }
}

static IOCP_PORT: OnceLock<GlobalPort> = OnceLock::new();

#[inline]
fn iocp_port() -> io::Result<&'static GlobalPort> {
    IOCP_PORT.get_or_try_init(GlobalPort::new)
}

fn iocp_start() -> io::Result<()> {
    let port = iocp_port()?;
    std::thread::spawn(move || {
        instrument!(compio_log::Level::TRACE, "iocp_start");
        let mut entries = Vec::with_capacity(CompletionPort::DEFAULT_CAPACITY);
        loop {
            let len = port.port.poll_raw(None, entries.spare_capacity_mut())?;
            unsafe { entries.set_len(len) };

            for entry in entries.drain(..) {
                // Any thin pointer is OK because we don't use the type of opcode.
                let overlapped_ptr: *mut Overlapped = entry.lpOverlapped.cast();
                let overlapped = unsafe { &*overlapped_ptr };
                if let Err(_e) = syscall!(
                    BOOL,
                    PostQueuedCompletionStatus(
                        overlapped.driver as _,
                        entry.dwNumberOfBytesTransferred,
                        entry.lpCompletionKey,
                        entry.lpOverlapped,
                    )
                ) {
                    error!(
                        "fail to dispatch entry ({}, {}, {:p}) to driver {:p}: {:?}",
                        entry.dwNumberOfBytesTransferred,
                        entry.lpCompletionKey,
                        entry.lpOverlapped,
                        overlapped.driver,
                        _e
                    );
                }
            }
        }
        #[allow(unreachable_code)]
        io::Result::Ok(())
    });
    Ok(())
}

static IOCP_INIT_ONCE: OnceLock<()> = OnceLock::new();

pub struct Port {
    port: CompletionPort,
    global_port: &'static GlobalPort,
}

impl Port {
    pub fn new() -> io::Result<Self> {
        IOCP_INIT_ONCE.get_or_try_init(iocp_start)?;

        Ok(Self {
            port: CompletionPort::new()?,
            global_port: iocp_port()?,
        })
    }

    pub fn attach(&self, fd: RawFd) -> io::Result<()> {
        self.global_port.attach(fd)
    }

    pub fn post(&self, res: io::Result<usize>, optr: *mut Overlapped) -> io::Result<()> {
        self.port.post(res, optr)
    }

    pub fn post_raw(&self, optr: *const Overlapped) -> io::Result<()> {
        self.port.post_raw(optr)
    }

    pub fn poll(&self, timeout: Option<Duration>) -> io::Result<impl Iterator<Item = Entry> + '_> {
        self.port.poll(timeout, None)
    }
}

impl AsRawHandle for Port {
    fn as_raw_handle(&self) -> RawHandle {
        self.port.as_raw_handle()
    }
}
