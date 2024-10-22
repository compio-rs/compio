#[cfg_attr(all(doc, docsrs), doc(cfg(all())))]
#[allow(unused_imports)]
pub use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io,
    num::NonZeroUsize,
    os::fd::BorrowedFd,
    pin::Pin,
    sync::Arc,
    task::Poll,
    time::Duration,
};

use compio_log::{instrument, trace};
use crossbeam_queue::SegQueue;
pub(crate) use libc::{sockaddr_storage, socklen_t};
use polling::{Event, Events, Poller};

use crate::{AsyncifyPool, Entry, Key, OutEntries, ProactorBuilder, op::Interest, syscall};

pub(crate) mod op;

/// Abstraction of operations.
pub trait OpCode {
    /// Perform the operation before submit, and return [`Decision`] to
    /// indicate whether submitting the operation to polling is required.
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision>;

    /// Perform the operation after received corresponding
    /// event. If this operation is blocking, the return value should be
    /// [`Poll::Ready`].
    fn on_event(self: Pin<&mut Self>, event: &Event) -> Poll<io::Result<usize>>;
}

/// Result of [`OpCode::pre_submit`].
pub enum Decision {
    /// Instant operation, no need to submit
    Completed(usize),
    /// Async operation, needs to submit
    Wait(WaitArg),
    /// Blocking operation, needs to be spawned in another thread
    Blocking(Event),
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

    /// Decide to spawn a blocking task with a dummy event.
    pub fn blocking_dummy() -> Self {
        Self::Blocking(Event::none(0))
    }

    /// Decide to spawn a blocking task with a readable event.
    pub fn blocking_readable(fd: RawFd) -> Self {
        Self::Blocking(Event::readable(fd as _))
    }

    /// Decide to spawn a blocking task with a writable event.
    pub fn blocking_writable(fd: RawFd) -> Self {
        Self::Blocking(Event::writable(fd as _))
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

    pub fn pop_interest(&mut self, event: &Event) -> Option<(usize, Interest)> {
        if event.readable {
            if let Some(user_data) = self.read_queue.pop_front() {
                return Some((user_data, Interest::Readable));
            }
        }
        if event.writable {
            if let Some(user_data) = self.write_queue.pop_front() {
                return Some((user_data, Interest::Writable));
            }
        }
        None
    }
}

/// Low-level driver of polling.
pub(crate) struct Driver {
    events: Events,
    poll: Arc<Poller>,
    registry: HashMap<RawFd, FdQueue>,
    cancelled: HashSet<usize>,
    pool: AsyncifyPool,
    pool_completed: Arc<SegQueue<Entry>>,
}

impl Driver {
    pub fn new(builder: &ProactorBuilder) -> io::Result<Self> {
        instrument!(compio_log::Level::TRACE, "new", ?builder);
        trace!("new poll driver");
        let entries = builder.capacity as usize; // for the sake of consistency, use u32 like iour
        let events = if entries == 0 {
            Events::new()
        } else {
            Events::with_capacity(NonZeroUsize::new(entries).unwrap())
        };

        let poll = Arc::new(Poller::new()?);

        Ok(Self {
            events,
            poll,
            registry: HashMap::new(),
            cancelled: HashSet::new(),
            pool: builder.create_or_get_thread_pool(),
            pool_completed: Arc::new(SegQueue::new()),
        })
    }

    pub fn create_op<T: crate::sys::OpCode + 'static>(&self, op: T) -> Key<T> {
        Key::new(self.as_raw_fd(), op)
    }

    /// # Safety
    /// The input fd should be valid.
    unsafe fn submit(&mut self, user_data: usize, arg: WaitArg) -> io::Result<()> {
        let need_add = !self.registry.contains_key(&arg.fd);
        let queue = self.registry.entry(arg.fd).or_default();
        queue.push_back_interest(user_data, arg.interest);
        // We use fd as the key.
        let event = queue.event(arg.fd as usize);
        if need_add {
            self.poll.add(arg.fd, event)?;
        } else {
            let fd = BorrowedFd::borrow_raw(arg.fd);
            self.poll.modify(fd, event)?;
        }
        Ok(())
    }

    pub fn attach(&mut self, _fd: RawFd) -> io::Result<()> {
        Ok(())
    }

    pub fn cancel<T>(&mut self, op: Key<T>) {
        self.cancelled.insert(op.user_data());
    }

    pub fn push<T: crate::sys::OpCode + 'static>(
        &mut self,
        op: &mut Key<T>,
    ) -> Poll<io::Result<usize>> {
        let user_data = op.user_data();
        let op_pin = op.as_op_pin();
        match op_pin.pre_submit() {
            Ok(Decision::Wait(arg)) => {
                // SAFETY: fd is from the OpCode.
                unsafe {
                    self.submit(user_data, arg)?;
                }
                Poll::Pending
            }
            Ok(Decision::Completed(res)) => Poll::Ready(Ok(res)),
            Ok(Decision::Blocking(event)) => {
                if self.push_blocking(user_data, event) {
                    Poll::Pending
                } else {
                    Poll::Ready(Err(io::Error::from_raw_os_error(libc::EBUSY)))
                }
            }
            Err(err) => Poll::Ready(Err(err)),
        }
    }

    fn push_blocking(&mut self, user_data: usize, event: Event) -> bool {
        let poll = self.poll.clone();
        let completed = self.pool_completed.clone();
        self.pool
            .dispatch(move || {
                let mut op = unsafe { Key::<dyn crate::sys::OpCode>::new_unchecked(user_data) };
                let op_pin = op.as_op_pin();
                let res = match op_pin.on_event(&event) {
                    Poll::Pending => unreachable!("this operation is not non-blocking"),
                    Poll::Ready(res) => res,
                };
                completed.push(Entry::new(user_data, res));
                poll.notify().ok();
            })
            .is_ok()
    }

    pub unsafe fn poll(
        &mut self,
        timeout: Option<Duration>,
        mut entries: OutEntries<impl Extend<usize>>,
    ) -> io::Result<()> {
        self.poll.wait(&mut self.events, timeout)?;
        if self.events.is_empty() && self.pool_completed.is_empty() && timeout.is_some() {
            return Err(io::Error::from_raw_os_error(libc::ETIMEDOUT));
        }
        while let Some(entry) = self.pool_completed.pop() {
            entries.extend(Some(entry));
        }
        for event in self.events.iter() {
            let fd = event.key as RawFd;
            let queue = self
                .registry
                .get_mut(&fd)
                .expect("the fd should be attached");
            if let Some((user_data, interest)) = queue.pop_interest(&event) {
                if self.cancelled.remove(&user_data) {
                    entries.extend(Some(entry_cancelled(user_data)));
                } else {
                    let mut op = Key::<dyn crate::sys::OpCode>::new_unchecked(user_data);
                    let op = op.as_op_pin();
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
            }
            let renew_event = queue.event(fd as _);
            let borrowed_fd = BorrowedFd::borrow_raw(fd);
            if !renew_event.readable && !renew_event.writable {
                self.poll.delete(borrowed_fd)?;
                self.registry.remove(&fd);
            } else {
                self.poll.modify(borrowed_fd, renew_event)?;
            }
        }
        Ok(())
    }

    pub fn handle(&self) -> io::Result<NotifyHandle> {
        Ok(NotifyHandle::new(self.poll.clone()))
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

/// A notify handle to the inner driver.
pub struct NotifyHandle {
    poll: Arc<Poller>,
}

impl NotifyHandle {
    fn new(poll: Arc<Poller>) -> Self {
        Self { poll }
    }

    /// Notify the inner driver.
    pub fn notify(&self) -> io::Result<()> {
        self.poll.notify()
    }
}
