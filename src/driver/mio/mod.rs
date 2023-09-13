#[doc(no_inline)]
pub use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::{
    collections::{HashMap, VecDeque},
    io,
    ops::ControlFlow,
    time::Duration,
};

pub(crate) use libc::{sockaddr_storage, socklen_t};
use mio::{
    event::{Event, Source},
    unix::SourceFd,
    Events, Interest, Poll, Token,
};

use crate::driver::{Entry, Poller};

pub(crate) mod op;

/// Abstraction of operations.
pub trait OpCode {
    /// Perform the operation before submit, and return [`Decision`] to
    /// indicate whether submitting the operation to mio is required.
    fn pre_submit(&mut self) -> io::Result<Decision>;

    /// Perform the operation after received corresponding
    /// event.
    fn on_event(&mut self, event: &Event) -> io::Result<ControlFlow<usize>>;
}

/// Result of [`OpCode::pre_submit`].
pub enum Decision {
    /// Instant operation, no need to submit
    Completed(usize),
    /// Async operation, needs to submit
    Wait(WaitArg),
}

impl Decision {
    /// Decide to wait for the given fd with the given interest.
    pub fn wait_for(fd: RawFd, interest: Interest) -> Self {
        Self::Wait(WaitArg { fd, interest })
    }

    /// Decide to wait for the given fd to be readable.
    pub fn wait_readable(fd: RawFd) -> Self {
        Self::wait_for(fd, Interest::READABLE)
    }

    /// Decide to wait for the given fd to be writable.
    pub fn wait_writable(fd: RawFd) -> Self {
        Self::wait_for(fd, Interest::WRITABLE)
    }
}

/// Meta of mio operations.
#[derive(Debug, Clone, Copy)]
pub struct WaitArg {
    fd: RawFd,
    interest: Interest,
}

/// Low-level driver of mio.
pub struct Driver {
    squeue: VecDeque<MioEntry>,
    events: Events,
    poll: Poll,
    waiting: HashMap<usize, WaitEntry>,
}

/// Entry in squeue
#[derive(Debug)]
struct MioEntry {
    op: *mut dyn OpCode,
    user_data: usize,
}

impl MioEntry {
    /// Safety: Caller mut guarantee that the op will live until it is
    /// completed.
    unsafe fn new(op: &mut (impl OpCode + 'static), user_data: usize) -> Self {
        Self {
            op: op as *mut dyn OpCode,
            user_data,
        }
    }

    fn op_mut(&mut self) -> &mut dyn OpCode {
        unsafe { &mut *self.op }
    }
}

/// Entry waiting for events
struct WaitEntry {
    op: *mut dyn OpCode,
    arg: WaitArg,
    user_data: usize,
}

impl WaitEntry {
    fn new(mio_entry: MioEntry, arg: WaitArg) -> Self {
        Self {
            op: mio_entry.op,
            arg,
            user_data: mio_entry.user_data,
        }
    }

    fn op_mut(&mut self) -> &mut dyn OpCode {
        unsafe { &mut *self.op }
    }
}

impl Driver {
    /// Create a new mio driver with 1024 entries.
    pub fn new() -> io::Result<Self> {
        Self::with_entries(1024)
    }

    /// Create a new mio driver with the given number of entries.
    pub fn with_entries(entries: u32) -> io::Result<Self> {
        let entries = entries as usize; // for the sake of consistency, use u32 like iour

        Ok(Self {
            squeue: VecDeque::with_capacity(entries),
            events: Events::with_capacity(entries),
            poll: Poll::new()?,
            waiting: HashMap::new(),
        })
    }
}

impl Driver {
    fn submit(&mut self, entry: MioEntry, arg: WaitArg) -> io::Result<()> {
        let token = Token(entry.user_data);

        SourceFd(&arg.fd).register(self.poll.registry(), token, arg.interest)?;

        // Only insert the entry after it was registered successfully
        self.waiting
            .insert(entry.user_data, WaitEntry::new(entry, arg));

        Ok(())
    }

    /// Register all operations in the squeue to mio.
    fn submit_squeue(&mut self, entries: &mut impl Extend<Entry>) -> io::Result<bool> {
        let mut extended = false;
        while let Some(mut entry) = self.squeue.pop_front() {
            match entry.op_mut().pre_submit() {
                Ok(Decision::Wait(arg)) => {
                    self.submit(entry, arg)?;
                }
                Ok(Decision::Completed(res)) => {
                    entries.extend(Some(Entry::new(entry.user_data, Ok(res))));
                    extended = true;
                }
                Err(err) => {
                    entries.extend(Some(Entry::new(entry.user_data, Err(err))));
                    extended = true;
                }
            }
        }

        Ok(extended)
    }

    /// Poll all events from mio, call `perform` on op and push them into
    /// cqueue.
    fn poll_impl(
        &mut self,
        timeout: Option<Duration>,
        entries: &mut impl Extend<Entry>,
    ) -> io::Result<()> {
        self.poll.poll(&mut self.events, timeout)?;
        if timeout.is_some() && self.events.is_empty() {
            return Err(io::Error::from_raw_os_error(libc::ETIMEDOUT));
        }
        for event in &self.events {
            let token = event.token();
            let entry = self
                .waiting
                .get_mut(&token.0)
                .expect("Unknown token returned by mio"); // XXX: Should this be silently ignored?
            let res = match entry.op_mut().on_event(event) {
                Ok(ControlFlow::Continue(_)) => continue,
                Ok(ControlFlow::Break(res)) => Ok(res),
                Err(err) => Err(err),
            };
            self.poll
                .registry()
                .deregister(&mut SourceFd(&entry.arg.fd))?;
            let entry = Entry::new(entry.user_data, res);
            entries.extend(Some(entry));
            self.waiting.remove(&token.0);
        }
        Ok(())
    }
}

impl Poller for Driver {
    fn attach(&mut self, _fd: RawFd) -> io::Result<()> {
        Ok(())
    }

    unsafe fn push(
        &mut self,
        op: &mut (impl OpCode + 'static),
        user_data: usize,
    ) -> io::Result<()> {
        self.squeue.push_back(MioEntry::new(op, user_data));
        Ok(())
    }

    fn cancel(&mut self, user_data: usize) {
        let Some(entry) = self.waiting.remove(&user_data) else {
            return;
        };
        self.poll
            .registry()
            .deregister(&mut SourceFd(&entry.arg.fd))
            .ok();
    }

    fn poll(
        &mut self,
        timeout: Option<Duration>,
        entries: &mut impl Extend<Entry>,
    ) -> io::Result<()> {
        let extended = self.submit_squeue(entries)?;
        if !extended {
            self.poll_impl(timeout, entries)?;

            // See if there are remaining entries.
            loop {
                if let Err(e) = self.poll_impl(Some(Duration::ZERO), entries) {
                    match e.kind() {
                        io::ErrorKind::TimedOut => break,
                        _ => return Err(e),
                    }
                }
            }
        }
        Ok(())
    }
}

impl AsRawFd for Driver {
    fn as_raw_fd(&self) -> RawFd {
        self.poll.as_raw_fd()
    }
}
