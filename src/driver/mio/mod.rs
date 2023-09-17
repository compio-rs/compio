#[doc(no_inline)]
pub use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::{
    collections::{HashMap, HashSet},
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

use crate::driver::{Entry, Operation, Poller};

pub(crate) mod op;

/// Abstraction of operations.
pub trait OpCode {
    /// Perform the operation before submit, and return [`Decision`] to
    /// indicate whether submitting the operation to mio is required.
    fn pre_submit(self: &mut Self) -> io::Result<Decision>;

    /// Perform the operation after received corresponding
    /// event.
    fn on_event(self: &mut Self, event: &Event) -> io::Result<ControlFlow<usize>>;
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
pub struct Driver<'arena> {
    events: Events,
    poll: Poll,
    waiting: HashMap<usize, WaitEntry<'arena>>,
    cancelled: HashSet<usize>,
}

/// Entry waiting for events
struct WaitEntry<'arena> {
    op: &'arena mut dyn OpCode,
    arg: WaitArg,
    user_data: usize,
}

impl<'arena> WaitEntry<'arena> {
    fn new(mio_entry: Operation<'arena>, arg: WaitArg) -> Self {
        let (op, user_data): (&'arena mut dyn OpCode, usize) = mio_entry.into();
        Self { op, arg, user_data }
    }
}

impl<'arena> Driver<'arena> {
    /// Create a new mio driver with 1024 entries.
    pub fn new() -> io::Result<Self> {
        Self::with_entries(1024)
    }

    /// Create a new mio driver with the given number of entries.
    pub fn with_entries(entries: u32) -> io::Result<Self> {
        let entries = entries as usize; // for the sake of consistency, use u32 like iour

        Ok(Self {
            events: Events::with_capacity(entries),
            poll: Poll::new()?,
            waiting: HashMap::new(),
            cancelled: HashSet::new(),
        })
    }

    fn submit(&mut self, entry: Operation<'arena>, arg: WaitArg) -> io::Result<()> {
        if !self.cancelled.remove(&entry.user_data) {
            let token = Token(entry.user_data);

            SourceFd(&arg.fd).register(self.poll.registry(), token, arg.interest)?;

            // Only insert the entry after it was registered successfully
            self.waiting
                .insert(entry.user_data, WaitEntry::new(entry, arg));
        }
        Ok(())
    }

    /// Register all operations in the squeue to mio.
    fn submit_squeue(
        &mut self,
        ops: &mut impl Iterator<Item = Operation<'arena>>,
        entries: &mut impl Extend<Entry>,
    ) -> io::Result<bool> {
        let mut extended = false;
        for mut entry in ops {
            // io buffers are Unpin so no need to pin
            match entry.opcode().pre_submit() {
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
        for event in &self.events {
            let token = event.token();
            let entry = self
                .waiting
                .get_mut(&token.0)
                .expect("Unknown token returned by mio"); // XXX: Should this be silently ignored?
            let res = match entry.op.on_event(event) {
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

impl<'arena> Poller<'arena> for Driver<'arena> {
    fn attach(&mut self, _fd: RawFd) -> io::Result<()> {
        Ok(())
    }

    fn cancel(&mut self, user_data: usize) {
        if let Some(entry) = self.waiting.remove(&user_data) {
            self.poll
                .registry()
                .deregister(&mut SourceFd(&entry.arg.fd))
                .ok();
        } else {
            self.cancelled.insert(user_data);
        }
    }

    unsafe fn poll(
        &mut self,
        timeout: Option<Duration>,
        ops: &mut impl Iterator<Item = Operation<'arena>>,
        entries: &mut impl Extend<Entry>,
    ) -> io::Result<()> {
        let extended = self.submit_squeue(ops, entries)?;
        if !extended {
            self.poll_impl(timeout, entries)?;
        }
        Ok(())
    }
}

impl AsRawFd for Driver<'_> {
    fn as_raw_fd(&self) -> RawFd {
        self.poll.as_raw_fd()
    }
}
