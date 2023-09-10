#[doc(no_inline)]
pub use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::{
    cell::RefCell, collections::VecDeque, io, mem::MaybeUninit, ops::ControlFlow, time::Duration,
};

pub(crate) use libc::{sockaddr_storage, socklen_t};
use mio::{
    event::{Event, Source},
    unix::SourceFd,
    Events, Interest, Poll, Token,
};
use slab::Slab;

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
pub struct Driver(RefCell<DriverInner>);

/// Inner state of [`Driver`].
struct DriverInner {
    squeue: VecDeque<MioEntry>,
    cqueue: VecDeque<Entry>,
    events: Events,
    poll: Poll,
    waiting: Slab<WaitEntry>,
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

        Ok(Self(RefCell::new(DriverInner {
            squeue: VecDeque::with_capacity(entries),
            cqueue: VecDeque::with_capacity(entries),
            events: Events::with_capacity(entries),
            poll: Poll::new()?,
            waiting: Slab::new(),
        })))
    }

    fn inner(&self) -> std::cell::RefMut<'_, DriverInner> {
        self.0.borrow_mut()
    }
}

impl DriverInner {
    fn submit(&mut self, entry: MioEntry, arg: WaitArg) -> io::Result<()> {
        let slot = self.waiting.vacant_entry();
        let token = Token(slot.key());

        SourceFd(&arg.fd).register(self.poll.registry(), token, arg.interest)?;

        // Only insert the entry after it was registered successfully
        slot.insert(WaitEntry::new(entry, arg));

        Ok(())
    }

    /// Register all operations in the squeue to mio.
    fn submit_squeue(&mut self) -> io::Result<()> {
        while let Some(mut entry) = self.squeue.pop_front() {
            match entry.op_mut().pre_submit() {
                Ok(Decision::Wait(arg)) => {
                    self.submit(entry, arg)?;
                }
                Ok(Decision::Completed(res)) => {
                    self.cqueue.push_back(Entry::new(entry.user_data, Ok(res)));
                }
                Err(err) => {
                    self.cqueue.push_back(Entry::new(entry.user_data, Err(err)));
                }
            }
        }

        Ok(())
    }

    /// Poll all events from mio, call `perform` on op and push them into
    /// cqueue.
    fn poll(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.poll.poll(&mut self.events, timeout)?;
        // println!("events: {:?}", self.events);
        for event in &self.events {
            let token = event.token();
            let entry = self
                .waiting
                .get_mut(token.0)
                .expect("Unknown token returned by mio"); // XXX: Should this be silently ignored?
            match entry.op_mut().on_event(event) {
                Ok(ControlFlow::Continue(_)) => {}
                Ok(ControlFlow::Break(res)) => {
                    self.cqueue.push_back(Entry::new(entry.user_data, Ok(res)));
                    self.waiting.remove(token.0);
                }
                Err(err) => {
                    self.cqueue.push_back(Entry::new(entry.user_data, Err(err)));
                    self.waiting.remove(token.0);
                }
            }
        }
        Ok(())
    }

    fn poll_completed(&mut self, entries: &mut [MaybeUninit<Entry>]) -> usize {
        let len = self.cqueue.len().min(entries.len());
        for entry in &mut entries[..len] {
            entry.write(self.cqueue.pop_front().unwrap());
        }
        len
    }
}

impl Poller for Driver {
    fn attach(&self, _fd: RawFd) -> io::Result<()> {
        Ok(())
    }

    unsafe fn push(&self, op: &mut (impl OpCode + 'static), user_data: usize) -> io::Result<()> {
        self.0
            .borrow_mut()
            .squeue
            .push_back(MioEntry::new(op, user_data));
        Ok(())
    }

    fn cancel(&self, user_data: usize) {
        let mut inner = self.inner();

        let Some(entry) = inner.waiting.try_remove(user_data) else {
            return;
        };
        inner
            .poll
            .registry()
            .deregister(&mut SourceFd(&entry.arg.fd))
            .ok();
    }

    fn poll(
        &self,
        timeout: Option<Duration>,
        entries: &mut [MaybeUninit<Entry>],
    ) -> io::Result<usize> {
        let mut inner = self.inner();

        inner.submit_squeue()?;
        if entries.is_empty() {
            return Ok(0);
        }
        if inner.poll_completed(entries) > 0 {
            return Ok(entries.len());
        }
        inner.poll(timeout)?;
        Ok(inner.poll_completed(entries))
    }
}
