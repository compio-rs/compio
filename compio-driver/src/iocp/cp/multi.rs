use std::{io, os::windows::io::AsRawHandle, sync::Arc, time::Duration};

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

    pub fn id(&self) -> usize {
        self.port.as_raw_handle() as _
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
