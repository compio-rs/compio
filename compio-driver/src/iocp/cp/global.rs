#[cfg(feature = "once_cell_try")]
use std::sync::OnceLock;
use std::{
    io,
    os::windows::io::{AsRawHandle, RawHandle},
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

use crossbeam_channel::{unbounded, Receiver, RecvTimeoutError, Sender, TryRecvError};
use crossbeam_skiplist::SkipMap;
#[cfg(not(feature = "once_cell_try"))]
use once_cell::sync::OnceCell as OnceLock;
use windows_sys::Win32::{
    Foundation::ERROR_TIMEOUT,
    System::IO::{PostQueuedCompletionStatus, OVERLAPPED},
};

use super::CompletionPort;
use crate::{syscall, Entry, Overlapped, RawFd};

struct GlobalPort {
    port: CompletionPort,
    drivers: SkipMap<usize, Sender<Entry>>,
}

impl GlobalPort {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            port: CompletionPort::new()?,
            drivers: SkipMap::new(),
        })
    }

    pub fn register(&self, driver: usize) -> Receiver<Entry> {
        let (sender, receiver) = unbounded();
        self.drivers.insert(driver, sender);
        receiver
    }

    pub fn deregister(&self, driver: usize) {
        self.drivers.remove(&driver);
    }

    pub fn attach(&self, fd: RawFd) -> io::Result<()> {
        self.port.attach(fd)
    }

    pub fn post<T: ?Sized>(
        &self,
        res: io::Result<usize>,
        optr: *mut Overlapped<T>,
    ) -> io::Result<()> {
        self.port.post(res, optr)
    }

    pub fn push(&self, driver: usize, entry: Entry) {
        if let Some(e) = self.drivers.get(&driver) {
            e.value().send(entry).ok(); // It's OK if the driver has been dropped.
        }
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
        loop {
            for (driver, entry) in port.port.poll(None, None)? {
                port.push(driver.0, entry);
            }
        }
        #[allow(unreachable_code)]
        io::Result::Ok(())
    });
    Ok(())
}

static DRIVER_COUNTER: AtomicUsize = AtomicUsize::new(0);
static IOCP_INIT_ONCE: OnceLock<()> = OnceLock::new();

pub struct Port {
    id: usize,
    port: &'static GlobalPort,
    receiver: Receiver<Entry>,
}

impl Port {
    pub fn new() -> io::Result<Self> {
        IOCP_INIT_ONCE.get_or_try_init(iocp_start)?;

        let id = DRIVER_COUNTER.fetch_add(1, Ordering::AcqRel);
        let port = iocp_port()?;
        let receiver = port.register(id);
        Ok(Self { id, port, receiver })
    }

    pub fn id(&self) -> PortId {
        PortId(self.id)
    }

    pub fn attach(&mut self, fd: RawFd) -> io::Result<()> {
        self.port.attach(fd)
    }

    pub fn handle(&self) -> PortHandle {
        PortHandle::new(self.port)
    }

    pub fn poll(&self, timeout: Option<Duration>) -> io::Result<impl Iterator<Item = Entry> + '_> {
        let e = if let Some(timeout) = timeout {
            match self.receiver.recv_timeout(timeout) {
                Ok(e) => e,
                Err(e) => match e {
                    RecvTimeoutError::Timeout => {
                        return Err(io::Error::from_raw_os_error(ERROR_TIMEOUT as _));
                    }
                    RecvTimeoutError::Disconnected => {
                        unreachable!("IOCP thread should not exit")
                    }
                },
            }
        } else {
            self.receiver.recv().expect("IOCP thread should not exit")
        };
        Ok(Some(e)
            .into_iter()
            .chain(std::iter::from_fn(|| match self.receiver.try_recv() {
                Ok(e) => Some(e),
                Err(e) => match e {
                    TryRecvError::Empty => None,
                    TryRecvError::Disconnected => unreachable!("IOCP thread should not exit"),
                },
            })))
    }
}

impl Drop for Port {
    fn drop(&mut self) {
        self.port.deregister(self.id);
    }
}

pub struct PortHandle {
    port: &'static GlobalPort,
}

impl PortHandle {
    fn new(port: &'static GlobalPort) -> Self {
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
pub struct PortId(usize);

impl PortId {
    /// Post raw entry to IOCP.
    pub fn post_raw(&self, transferred: u32, key: usize, optr: *mut OVERLAPPED) -> io::Result<()> {
        syscall!(
            BOOL,
            PostQueuedCompletionStatus(iocp_port()?.as_raw_handle() as _, transferred, key, optr)
        )?;
        Ok(())
    }
}
