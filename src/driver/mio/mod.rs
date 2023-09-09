#[doc(no_inline)]
pub use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::{cell::RefCell, io, mem::MaybeUninit, ops::DerefMut, time::Duration};

pub(crate) use libc::{sockaddr_storage, socklen_t};
use mio::{
    event::{Event, Source},
    unix::SourceFd,
    Events, Interest, Poll, Token,
};
use slab::Slab;

use crate::driver::{queue_with_capacity, Entry, Poller, Queue};

pub(crate) mod fs;
pub(crate) mod net;
pub(crate) mod op;

/// Helper macro to execute a system call that returns an `io::Result`.
macro_rules! syscall {
    ($fn: ident ( $($arg: expr),* $(,)* ) ) => {{
        #[allow(unused_unsafe)]
        let res = unsafe { libc::$fn($($arg, )*) };
        if res == -1 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(res as _)
        }
    }};
}

pub(crate) use syscall;

/// Abstraction of operations.
pub trait OpCode {
    /// Perform the operation before submit, and return [`Decision`] to
    /// indicate whether submitting the operation to mio is required.
    fn pre_submit(&mut self) -> io::Result<Decision>;

    /// Perform the operation after received corresponding
    /// event.
    fn on_event(&mut self, event: &Event) -> io::Result<usize>;
}

/// Result of [`OpCode::pre_submit`].
pub enum Decision {
    /// Instant operation, no need to submit
    Completed(usize),
    /// Async operation, needs to submit
    Wait(OpMeta),
}

impl Decision {
    /// Decide to complete the operation with the given result.
    pub fn complete(result: usize) -> Self {
        Self::Completed(result)
    }

    /// Decide to wait for the given fd with the given interest.
    pub fn wait_for(fd: RawFd, interest: Interest) -> Self {
        Self::Wait(OpMeta { fd, interest })
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
pub struct OpMeta {
    fd: RawFd,
    interest: Interest,
}

/// Low-level driver of mio.
pub struct Driver {
    inner: RefCell<DriverInner>,
    squeue: Queue<MioEntry>,
    cqueue: Queue<Entry>,
}

/// Inner state of [`Driver`].
struct DriverInner {
    events: Events,
    poll: Poll,
    registered: Slab<MioEntry>,
}

/// Internal representation of operation being submitted into the driver.
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

    fn op(&self) -> &dyn OpCode {
        unsafe { &*self.op }
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
            squeue: queue_with_capacity(entries),
            cqueue: queue_with_capacity(entries),
            inner: RefCell::new(DriverInner {
                events: Events::with_capacity(entries),
                poll: Poll::new()?,
                registered: Slab::new(),
            }),
        })
    }

    /// Register all operations in the squeue to mio.
    fn submit_squeue(&self) -> io::Result<()> {
        self.with_inner(|inner| {
            while let Some(mut entry) = self.squeue.pop() {
                match entry.op_mut().pre_submit() {
                    Ok(Decision::Wait(meta)) => {
                        inner.submit(entry, meta)?;
                    }
                    Ok(Decision::Completed(res)) => {
                        self.cqueue.push(Entry::new(entry.user_data, Ok(res)));
                    }
                    Err(err) => {
                        self.cqueue.push(Entry::new(entry.user_data, Err(err)));
                    }
                }
            }

            Ok(())
        })
    }

    /// Poll all events from mio, call `perform` on op and push them into
    /// cqueue.
    fn poll(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.with_inner(|inner| {
            inner.poll.poll(&mut inner.events, timeout)?;

            for event in &inner.events {
                let token = event.token();
                let mut entry = inner
                    .registered
                    .try_remove(token.0)
                    .expect("Unknown token returned by mio"); // XXX: Should this be silently ignored?
                let res = entry.op_mut().on_event(event);

                self.cqueue.push(Entry::new(entry.user_data, res))
            }
            Ok(())
        })
    }

    fn poll_completed(&self, entries: &mut [MaybeUninit<Entry>]) -> usize {
        let len = self.cqueue.len().min(entries.len());
        for entry in &mut entries[..len] {
            entry.write(self.cqueue.pop().unwrap());
        }
        len
    }

    fn with_inner<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut DriverInner) -> R,
    {
        f(self.inner.borrow_mut().deref_mut())
    }
}

impl DriverInner {
    fn submit(&mut self, entry: MioEntry, meta: OpMeta) -> io::Result<()> {
        let slot = self.registered.vacant_entry();
        let token = Token(slot.key());

        SourceFd(&meta.fd).register(self.poll.registry(), token, meta.interest)?;

        // Only insert the entry after it was registered successfully
        slot.insert(entry);

        Ok(())
    }
}

impl Poller for Driver {
    fn attach(&self, _fd: RawFd) -> io::Result<()> {
        Ok(())
    }

    unsafe fn push(&self, op: &mut (impl OpCode + 'static), user_data: usize) -> io::Result<()> {
        self.squeue.push(MioEntry::new(op, user_data));
        Ok(())
    }

    fn post(&self, user_data: usize, result: usize) -> io::Result<()> {
        self.cqueue.push(Entry::new(user_data, Ok(result)));
        Ok(())
    }

    fn poll(
        &self,
        timeout: Option<Duration>,
        entries: &mut [MaybeUninit<Entry>],
    ) -> io::Result<usize> {
        self.submit_squeue()?;
        if entries.is_empty() {
            return Ok(0);
        }
        if self.poll_completed(entries) > 0 {
            return Ok(entries.len());
        }
        self.poll(timeout)?;
        Ok(self.poll_completed(entries))
    }
}
