use std::{
    io,
    os::windows::io::{AsRawHandle, RawHandle},
    sync::Arc,
    time::Duration,
};

use super::CompletionPort;
use crate::{Entry, Overlapped, RawFd};

pub struct Port {
    port: Arc<CompletionPort>,
}

impl Port {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            port: Arc::new(CompletionPort::new()?),
        })
    }

    pub fn attach(&mut self, fd: RawFd) -> io::Result<()> {
        self.port.attach(fd)
    }

    pub fn handle(&self) -> PortHandle {
        PortHandle::new(self.port.clone())
    }

    pub fn post_raw(&self, optr: *const Overlapped) -> io::Result<()> {
        self.port.post_raw(optr)
    }

    pub fn poll(&self, timeout: Option<Duration>) -> io::Result<impl Iterator<Item = Entry> + '_> {
        let current_id = self.as_raw_handle() as _;
        self.port.poll(timeout, Some(current_id))
    }
}

impl AsRawHandle for Port {
    fn as_raw_handle(&self) -> RawHandle {
        self.port.as_raw_handle()
    }
}

pub struct PortHandle {
    port: Arc<CompletionPort>,
}

impl PortHandle {
    fn new(port: Arc<CompletionPort>) -> Self {
        Self { port }
    }

    pub fn post(&self, res: io::Result<usize>, optr: *mut Overlapped) -> io::Result<()> {
        self.port.post(res, optr)
    }

    pub fn post_raw(&self, optr: *const Overlapped) -> io::Result<()> {
        self.port.post_raw(optr)
    }
}
