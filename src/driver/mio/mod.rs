#[doc(no_inline)]
pub use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::{cell::RefCell, io, mem::MaybeUninit, ops::DerefMut, time::Duration};

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

/// Abstraction of mio operations.
pub trait OpCode {
    fn interests(&self) -> Interest;
    fn source_fd(&self) -> SourceFd<'_>;
    fn perform(&mut self, event: &Event) -> io::Result<usize>;
}

/// Low-level driver of mio.
pub struct Driver {
    inner: RefCell<DriverInner>,
    squeue: Queue<MioEntry>,
    cqueue: Queue<Entry>,
}

/// Inner state of [`Driver`].
pub struct DriverInner {
    events: Events,
    poll: Poll,
    registered: Slab<MioEntry>,
}

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

    fn op_mut(&self) -> &mut dyn OpCode {
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
            while let Some(entry) = self.squeue.pop() {
                inner.submit(entry)?;
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
                let entry = inner
                    .registered
                    .try_remove(token.0)
                    .expect("Unknown token returned by mio");
                let res = entry.op_mut().perform(event);

                self.cqueue.push(Entry::new(token.into(), res))
            }
            Ok(())
        })
    }

    fn get_completed(&self, entries: &mut [MaybeUninit<Entry>]) -> usize {
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
    fn submit(&mut self, entry: MioEntry) -> io::Result<()> {
        let slot = self.registered.vacant_entry();
        let token = Token(slot.key());

        entry
            .op()
            .source_fd()
            .register(self.poll.registry(), token, entry.op().interests())?;

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
        todo!()
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
        if self.get_completed(entries) > 0 {
            return Ok(entries.len());
        }
        self.poll(timeout)?;
        Ok(self.get_completed(entries))
    }
}
