use std::{
    io,
    os::windows::io::{AsRawHandle, RawHandle},
    time::Duration,
};

use super::{CompletionPort, RawEntry};
use crate::{Overlapped, RawFd};

pub struct Port {
    port: CompletionPort,
}

impl Port {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            port: CompletionPort::new()?,
        })
    }

    pub fn attach(&self, fd: RawFd) -> io::Result<()> {
        self.port.attach(fd)
    }

    pub fn post(&self, res: io::Result<usize>, optr: *mut Overlapped) -> io::Result<()> {
        self.port.post(res, optr)
    }

    pub fn post_raw(&self, optr: *const Overlapped) -> io::Result<()> {
        self.port.post_raw(optr)
    }

    pub fn poll(
        &self,
        timeout: Option<Duration>,
    ) -> io::Result<impl Iterator<Item = RawEntry> + '_> {
        let current_id = self.as_raw_handle() as _;
        self.port.poll(timeout, Some(current_id))
    }
}

impl AsRawHandle for Port {
    fn as_raw_handle(&self) -> RawHandle {
        self.port.as_raw_handle()
    }
}
