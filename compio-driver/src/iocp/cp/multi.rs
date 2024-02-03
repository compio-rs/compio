use std::{io, os::windows::io::AsRawHandle, sync::Arc, time::Duration};

use windows_sys::Win32::{
    Foundation::HANDLE,
    System::IO::{PostQueuedCompletionStatus, OVERLAPPED},
};

use super::CompletionPort;
use crate::{syscall, Entry, Overlapped, RawFd};

pub struct Port {
    port: Arc<CompletionPort>,
}

impl Port {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            port: Arc::new(CompletionPort::new()?),
        })
    }

    pub fn id(&self) -> PortId {
        PortId(self.port.as_raw_handle() as _)
    }

    pub fn attach(&mut self, fd: RawFd) -> io::Result<()> {
        self.port.attach(fd)
    }

    pub fn handle(&self) -> PortHandle {
        PortHandle::new(self.port.clone())
    }

    pub fn poll(&self, timeout: Option<Duration>) -> io::Result<impl Iterator<Item = Entry> + '_> {
        let current_id = self.id();
        self.port.poll(timeout, Some(current_id)).map(move |it| {
            it.filter_map(
                move |(id, entry)| {
                    if id == current_id { Some(entry) } else { None }
                },
            )
        })
    }
}

pub struct PortHandle {
    port: Arc<CompletionPort>,
}

impl PortHandle {
    fn new(port: Arc<CompletionPort>) -> Self {
        Self { port }
    }

    pub fn post<T: ?Sized>(
        &self,
        res: io::Result<usize>,
        optr: *mut Overlapped<T>,
    ) -> io::Result<()> {
        self.port.post(res, optr)
    }
}

/// The unique ID of IOCP driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortId(HANDLE);

impl PortId {
    /// Post raw entry to IOCP.
    pub fn post_raw(&self, transferred: u32, key: usize, optr: *mut OVERLAPPED) -> io::Result<()> {
        syscall!(
            BOOL,
            PostQueuedCompletionStatus(self.0, transferred, key, optr)
        )?;
        Ok(())
    }
}
