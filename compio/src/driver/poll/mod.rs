#[doc(no_inline)]
pub use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io,
    num::NonZeroUsize,
    os::fd::BorrowedFd,
    pin::Pin,
    task::Poll,
    time::Duration,
};

pub(crate) use libc::{sockaddr_storage, socklen_t};
use polling::{Event, Events, Poller};
use slab::Slab;

use crate::{driver::Entry, syscall};

pub(crate) mod op;
pub(crate) use crate::driver::unix::RawOp;

/// Abstraction of operations.
pub trait OpCode {
    /// Perform the operation before submit, and return [`Decision`] to
    /// indicate whether submitting the operation to polling is required.
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision>;

    /// Perform the operation after received corresponding
    /// event.
    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>>;
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
        Self::wait_for(fd, Interest::Readable)
    }

    /// Decide to wait for the given fd to be writable.
    pub fn wait_writable(fd: RawFd) -> Self {
        Self::wait_for(fd, Interest::Writable)
    }
}

/// Meta of polling operations.
#[derive(Debug, Clone, Copy)]
pub struct WaitArg {
    /// The raw fd of the operation.
    pub fd: RawFd,
    /// The interest to be registered.
    pub interest: Interest,
}

/// The interest of the operation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interest {
    /// Represents a read operation.
    Readable,
    /// Represents a write operation.
    Writable,
}

#[derive(Debug, Default)]
struct FdQueue {
    read_queue: VecDeque<usize>,
    write_queue: VecDeque<usize>,
}

impl FdQueue {
    pub fn push_back_interest(&mut self, user_data: usize, interest: Interest) {
        match interest {
            Interest::Readable => self.read_queue.push_back(user_data),
            Interest::Writable => self.write_queue.push_back(user_data),
        }
    }

    pub fn push_front_interest(&mut self, user_data: usize, interest: Interest) {
        match interest {
            Interest::Readable => self.read_queue.push_front(user_data),
            Interest::Writable => self.write_queue.push_front(user_data),
        }
    }

    pub fn event(&self, key: usize) -> Event {
        let mut event = Event::all(key);
        event.readable = !self.read_queue.is_empty();
        event.writable = !self.write_queue.is_empty();
        event
    }

    pub fn pop_interest(&mut self, event: &Event) -> (usize, Interest) {
        if event.readable {
            if let Some(user_data) = self.read_queue.pop_front() {
                return (user_data, Interest::Readable);
            }
        }
        if event.writable {
            if let Some(user_data) = self.write_queue.pop_front() {
                return (user_data, Interest::Writable);
            }
        }
        unreachable!("should not receive event when no interest")
    }
}

/// Low-level driver of polling.
pub(crate) struct Driver {
    events: Events,
    poll: Poller,
    registry: HashMap<RawFd, FdQueue>,
    cancelled: HashSet<usize>,
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
            registry: HashMap::new(),
            cancelled: HashSet::new(),
        })
    }

    fn submit(&mut self, user_data: usize, arg: WaitArg) -> io::Result<bool> {
        if self.cancelled.remove(&user_data) {
            Ok(false)
        } else {
            let queue = self
                .registry
                .get_mut(&arg.fd)
                .expect("the fd should be attached");
            queue.push_back_interest(user_data, arg.interest);
            // We use fd as the key.
            let event = queue.event(arg.fd as usize);
            unsafe {
                let fd = BorrowedFd::borrow_raw(arg.fd);
                self.poll.modify(fd, event)?;
            }
            Ok(true)
        }
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
                    let succeeded = self.submit(user_data, arg)?;
                    if !succeeded {
                        entries.extend(Some(entry_cancelled(user_data)));
                        extended = true;
                    }
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
            let fd = event.key as RawFd;
            let queue = self
                .registry
                .get_mut(&fd)
                .expect("the fd should be attached");
            let (user_data, interest) = queue.pop_interest(&event);
            if self.cancelled.remove(&user_data) {
                entries.extend(Some(entry_cancelled(user_data)));
            } else {
                let op = registry[user_data].as_pin();
                let res = match op.on_event(&event) {
                    Poll::Pending => {
                        // The operation should go back to the front.
                        queue.push_front_interest(user_data, interest);
                        None
                    }
                    Poll::Ready(res) => Some(res),
                };
                if let Some(res) = res {
                    let entry = Entry::new(user_data, res);
                    entries.extend(Some(entry));
                }
            }
            let renew_event = queue.event(fd as _);
            unsafe {
                let fd = BorrowedFd::borrow_raw(fd);
                self.poll.modify(fd, renew_event)?;
            }
        }
        Ok(())
    }

    pub fn attach(&mut self, fd: RawFd) -> io::Result<()> {
        if cfg!(any(target_os = "linux", target_os = "android")) {
            let mut stat = unsafe { std::mem::zeroed() };
            syscall!(fstat(fd, &mut stat))?;
            if matches!(stat.st_mode & libc::S_IFMT, libc::S_IFREG | libc::S_IFDIR) {
                return Ok(());
            }
        }
        self.registry.entry(fd).or_default();
        unsafe {
            self.poll.add(fd, Event::none(0))?;
        }
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
        let extended = self.submit_squeue(ops, entries, registry)?;
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

impl Drop for Driver {
    fn drop(&mut self) {
        for fd in self.registry.keys() {
            unsafe {
                let fd = BorrowedFd::borrow_raw(*fd);
                self.poll.delete(fd).ok();
            }
        }
    }
}

fn entry_cancelled(user_data: usize) -> Entry {
    Entry::new(
        user_data,
        Err(io::Error::from_raw_os_error(libc::ETIMEDOUT)),
    )
}
