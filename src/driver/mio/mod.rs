#[doc(no_inline)]
pub use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io,
    ops::ControlFlow,
    pin::Pin,
    time::Duration,
};

pub(crate) use libc::{sockaddr_storage, socklen_t};
use mio::{
    event::{Event, Source},
    unix::SourceFd,
    Events, Interest, Poll, Token,
};
use slab::Slab;

use crate::driver::Entry;

pub(crate) mod op;
pub(crate) use crate::driver::unix::RawOp;

/// Abstraction of operations.
pub trait OpCode {
    /// Perform the operation before submit, and return [`Decision`] to
    /// indicate whether submitting the operation to mio is required.
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision>;

    /// Perform the operation after received corresponding
    /// event.
    fn on_event(self: Pin<&mut Self>, event: &Event) -> io::Result<ControlFlow<usize>>;
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
pub(crate) struct Driver {
    events: Events,
    poll: Poll,
    waiting: HashMap<usize, WaitEntry>,
    cancelled: HashSet<usize>,
    cancel_queue: VecDeque<usize>,
}

/// Entry waiting for events
struct WaitEntry {
    arg: WaitArg,
    user_data: usize,
}

impl WaitEntry {
    fn new(user_data: usize, arg: WaitArg) -> Self {
        Self { arg, user_data }
    }
}

impl Driver {
    pub fn new(entries: u32) -> io::Result<Self> {
        let entries = entries as usize; // for the sake of consistency, use u32 like iour

        Ok(Self {
            events: Events::with_capacity(entries),
            poll: Poll::new()?,
            waiting: HashMap::new(),
            cancelled: HashSet::new(),
            cancel_queue: VecDeque::new(),
        })
    }
}

impl Driver {
    fn submit(&mut self, user_data: usize, arg: WaitArg) -> io::Result<()> {
        if self.cancelled.remove(&user_data) {
            self.cancel_queue.push_back(user_data);
        } else {
            let token = Token(user_data);

            SourceFd(&arg.fd).register(self.poll.registry(), token, arg.interest)?;

            // Only insert the entry after it was registered successfully
            self.waiting
                .insert(user_data, WaitEntry::new(user_data, arg));
        }
        Ok(())
    }

    /// Register all operations in the squeue to mio.
    fn submit_squeue(
        &mut self,
        ops: &mut impl Iterator<Item = usize>,
        entries: &mut impl Extend<Entry>,
        registry: &mut Slab<RawOp>,
    ) -> io::Result<bool> {
        let mut extended = false;
        for user_data in ops {
            let op = registry[user_data].as_pin();
            match op.pre_submit() {
                Ok(Decision::Wait(arg)) => {
                    self.submit(user_data, arg)?;
                }
                Ok(Decision::Completed(res)) => {
                    entries.extend(Some(Entry::new(user_data, Ok(res))));
                    extended = true;
                }
                Err(err) => {
                    entries.extend(Some(Entry::new(user_data, Err(err))));
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
        registry: &mut Slab<RawOp>,
    ) -> io::Result<()> {
        self.poll.poll(&mut self.events, timeout)?;
        for event in &self.events {
            let token = event.token();
            let entry = self
                .waiting
                .get_mut(&token.0)
                .expect("Unknown token returned by mio"); // XXX: Should this be silently ignored?
            let op = registry[entry.user_data].as_pin();
            let res = match op.on_event(event) {
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

    fn poll_cancel(&mut self, entries: &mut impl Extend<Entry>) {
        entries.extend(self.cancel_queue.drain(..).map(|user_data| {
            Entry::new(
                user_data,
                Err(io::Error::from_raw_os_error(libc::ETIMEDOUT)),
            )
        }))
    }

    pub fn attach(&mut self, _fd: RawFd) -> io::Result<()> {
        Ok(())
    }

    pub fn cancel(&mut self, user_data: usize) {
        if let Some(entry) = self.waiting.remove(&user_data) {
            self.poll
                .registry()
                .deregister(&mut SourceFd(&entry.arg.fd))
                .ok();
            self.cancel_queue.push_back(user_data);
        } else {
            self.cancelled.insert(user_data);
        }
    }

    pub unsafe fn poll(
        &mut self,
        timeout: Option<Duration>,
        ops: &mut impl Iterator<Item = usize>,
        entries: &mut impl Extend<Entry>,
        registry: &mut Slab<RawOp>,
    ) -> io::Result<()> {
        let extended = self.submit_squeue(ops, entries, registry)?;
        if !extended {
            self.poll_impl(timeout, entries, registry)?;
        }
        self.poll_cancel(entries);
        Ok(())
    }
}

impl AsRawFd for Driver {
    fn as_raw_fd(&self) -> RawFd {
        self.poll.as_raw_fd()
    }
}
