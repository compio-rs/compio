#[doc(no_inline)]
pub use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::{
    collections::{HashMap, VecDeque},
    io,
    mem::MaybeUninit,
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
    cqueue: VecDeque<Entry>,
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
            cqueue: VecDeque::with_capacity(entries),
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
    fn poll_impl(
        &mut self,
        timeout: Option<Duration>,
        entries: &mut [MaybeUninit<Entry>],
    ) -> io::Result<usize> {
        let entries = EntriesVec::new(entries);
        self.poll.poll(&mut self.events, timeout)?;
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
            if let Some(entry) = entries.push_back(entry) {
                self.cqueue.push_back(entry);
            }
            self.waiting.remove(&token.0);
        }
        Ok(entries.entries_len())
    }

    fn poll_completed(&mut self, entries: &mut [MaybeUninit<Entry>]) -> usize {
        let len = self.cqueue.len().min(entries.len());
        for (entry, cqe) in entries.iter_mut().zip(self.cqueue.drain(..len)) {
            entry.write(cqe);
        }
        len
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
        entries: &mut [MaybeUninit<Entry>],
    ) -> io::Result<usize> {
        self.submit_squeue()?;
        if entries.is_empty() {
            return Ok(0);
        }
        let len = self.poll_completed(entries);
        if len > 0 {
            return Ok(len);
        }
        self.poll_impl(timeout, entries)
    }
}

impl AsRawFd for Driver {
    fn as_raw_fd(&self) -> RawFd {
        self.poll.as_raw_fd()
    }
}

struct EntriesVec<'a> {
    entries: &'a mut [MaybeUninit<Entry>],
    index: usize,
}

impl<'a> EntriesVec<'a> {
    pub fn new(entries: &'a mut [MaybeUninit<Entry>]) -> Self {
        Self { entries, index: 0 }
    }

    pub fn push_back(&mut self, entry: Entry) -> Option<Entry> {
        if self.index < self.entries.len() {
            self.entries[self.index].write(entry);
            self.index += 1;
            None
        } else {
            Some(entry)
        }
    }

    pub fn entries_len(&self) -> usize {
        self.index
    }
}
