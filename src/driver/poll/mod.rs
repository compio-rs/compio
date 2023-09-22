#[doc(no_inline)]
pub use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::{
    collections::{HashSet, VecDeque},
    io,
    num::NonZeroUsize,
    ops::ControlFlow,
    pin::Pin,
    time::Duration,
};

pub(crate) use libc::{sockaddr_storage, socklen_t};
use polling::{Event, Events, Poller};
use slab::Slab;

use crate::driver::Entry;

pub(crate) mod op;
pub(crate) use crate::driver::unix::RawOp;

/// Abstraction of operations.
pub trait OpCode {
    /// Perform the operation before submit, and return [`Decision`] to
    /// indicate whether submitting the operation to polling is required.
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
    pub fn wait_for(fd: RawFd, readable: bool, writable: bool) -> Self {
        Self::Wait(WaitArg {
            fd,
            readable,
            writable,
        })
    }

    /// Decide to wait for the given fd to be readable.
    pub fn wait_readable(fd: RawFd) -> Self {
        Self::wait_for(fd, true, false)
    }

    /// Decide to wait for the given fd to be writable.
    pub fn wait_writable(fd: RawFd) -> Self {
        Self::wait_for(fd, false, true)
    }
}

/// Meta of polling operations.
#[derive(Debug, Clone, Copy)]
pub struct WaitArg {
    fd: RawFd,
    readable: bool,
    writable: bool,
}

/// Low-level driver of polling.
pub(crate) struct Driver {
    events: Events,
    poll: Poller,
    cancelled: HashSet<usize>,
    cancel_queue: VecDeque<usize>,
}

impl Driver {
    pub fn new(entries: u32) -> io::Result<Self> {
        let entries = entries as usize; // for the sake of consistency, use u32 like iour
        let events = if entries == 0 {
            Events::new()
        } else {
            Events::with_capacity(NonZeroUsize::new(entries).unwrap())
        };

        Ok(Self {
            events,
            poll: Poller::new()?,
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
            let mut event = Event::none(user_data);
            event.readable = arg.readable;
            event.writable = arg.writable;
            unsafe {
                self.poll.add(arg.fd, event)?;
            }
        }
        Ok(())
    }

    /// Register all operations in the squeue to polling.
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

    /// Poll all events from polling, call `perform` on op and push them into
    /// cqueue.
    fn poll_impl(
        &mut self,
        timeout: Option<Duration>,
        entries: &mut impl Extend<Entry>,
        registry: &mut Slab<RawOp>,
    ) -> io::Result<()> {
        self.poll.wait(&mut self.events, timeout)?;
        if self.events.is_empty() && timeout.is_some() {
            return Err(io::Error::from_raw_os_error(libc::ETIMEDOUT));
        }
        for event in self.events.iter() {
            if self.cancelled.remove(&event.key) {
                self.cancel_queue.push_back(event.key);
            } else {
                let op = registry[event.key].as_pin();
                let res = match op.on_event(&event) {
                    Ok(ControlFlow::Continue(_)) => continue,
                    Ok(ControlFlow::Break(res)) => Ok(res),
                    Err(err) => Err(err),
                };
                let entry = Entry::new(event.key, res);
                entries.extend(Some(entry));
            }
        }
        Ok(())
    }

    fn poll_cancel(&mut self, entries: &mut impl Extend<Entry>) -> bool {
        let has_cancel = !self.cancel_queue.is_empty();
        if has_cancel {
            entries.extend(self.cancel_queue.drain(..).map(|user_data| {
                Entry::new(
                    user_data,
                    Err(io::Error::from_raw_os_error(libc::ETIMEDOUT)),
                )
            }))
        }
        has_cancel
    }

    pub fn attach(&mut self, _fd: RawFd) -> io::Result<()> {
        Ok(())
    }

    pub fn cancel(&mut self, user_data: usize, _registry: &mut Slab<RawOp>) {
        self.cancelled.insert(user_data);
    }

    pub unsafe fn poll(
        &mut self,
        timeout: Option<Duration>,
        ops: &mut impl Iterator<Item = usize>,
        entries: &mut impl Extend<Entry>,
        registry: &mut Slab<RawOp>,
    ) -> io::Result<()> {
        let mut extended = self.submit_squeue(ops, entries, registry)?;
        extended |= self.poll_cancel(entries);
        if !extended {
            self.poll_impl(timeout, entries, registry)?;
        }
        Ok(())
    }
}

impl AsRawFd for Driver {
    fn as_raw_fd(&self) -> RawFd {
        self.poll.as_raw_fd()
    }
}
