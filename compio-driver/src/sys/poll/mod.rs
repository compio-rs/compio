#[cfg_attr(all(doc, docsrs), doc(cfg(all())))]
#[allow(unused_imports)]
pub use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd};
#[cfg(aio)]
use std::ptr::NonNull;
use std::{
    collections::{HashMap, VecDeque},
    io,
    num::NonZeroUsize,
    pin::Pin,
    sync::Arc,
    task::{Poll, Wake, Waker},
    time::Duration,
};

use compio_log::{instrument, trace};
use crossbeam_queue::SegQueue;
use polling::{Event, Events, Poller};

use crate::{
    AsyncifyPool, BufferPool, DriverType, Entry, ErasedKey, ProactorBuilder,
    key::{BorrowedKey, Key, RefExt},
    op::Interest,
    syscall,
};

pub(crate) mod op;

/// Extra data for RawOp.
///
/// Polling doesn't need any extra data in RawOp so it's empty.
#[allow(dead_code)]
#[derive(Default)]
pub struct Extra {}

#[allow(dead_code)]
impl Extra {
    pub fn new(_: RawFd) -> Self {
        Self {}
    }
}

/// Abstraction of operations.
pub trait OpCode {
    /// Perform the operation before submit, and return [`Decision`] to
    /// indicate whether submitting the operation to polling is required.
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision>;

    /// Get the operation type when an event is occurred.
    fn op_type(self: Pin<&mut Self>) -> Option<OpType> {
        None
    }

    /// Perform the operation after received corresponding
    /// event. If this operation is blocking, the return value should be
    /// [`Poll::Ready`].
    fn operate(self: Pin<&mut Self>) -> Poll<io::Result<usize>>;
}

pub use OpCode as PollOpCode;

/// Result of [`OpCode::pre_submit`].
#[non_exhaustive]
pub enum Decision {
    /// Instant operation, no need to submit
    Completed(usize),
    /// Async operation, needs to submit
    Wait(WaitArg),
    /// Blocking operation, needs to be spawned in another thread
    Blocking,
    /// AIO operation, needs to be spawned to the kernel.
    #[cfg(aio)]
    Aio(AioControl),
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

    /// Decide to spawn an AIO operation. `submit` is a method like `aio_read`.
    #[cfg(aio)]
    pub fn aio(
        cb: &mut libc::aiocb,
        submit: unsafe extern "C" fn(*mut libc::aiocb) -> i32,
    ) -> Self {
        Self::Aio(AioControl {
            aiocbp: NonNull::from(cb),
            submit,
        })
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

/// Meta of AIO operations.
#[cfg(aio)]
#[derive(Debug, Clone, Copy)]
pub struct AioControl {
    /// Pointer of the control block.
    pub aiocbp: NonNull<libc::aiocb>,
    /// The aio_* submit function.
    pub submit: unsafe extern "C" fn(*mut libc::aiocb) -> i32,
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

    pub fn remove(&mut self, user_data: usize) {
        self.read_queue.retain(|&k| k != user_data);
        self.write_queue.retain(|&k| k != user_data);
    }

    pub fn event(&self) -> Event {
        let mut event = Event::none(0);
        if let Some(&key) = self.read_queue.front() {
            event.readable = true;
            event.key = key;
        }
        if let Some(&key) = self.write_queue.front() {
            event.writable = true;
            event.key = key;
        }
        event
    }

    pub fn pop_interest(&mut self, event: &Event) -> Option<(usize, Interest)> {
        if event.readable
            && let Some(user_data) = self.read_queue.pop_front()
        {
            return Some((user_data, Interest::Readable));
        }
        if event.writable
            && let Some(user_data) = self.write_queue.pop_front()
        {
            return Some((user_data, Interest::Writable));
        }
        None
    }
}

/// Represents the filter type of kqueue. `polling` crate doesn't expose such
/// API, and we need to know about it when `cancel` is called.
#[non_exhaustive]
pub enum OpType {
    /// The operation polls an fd.
    Fd(RawFd),
    /// The operation submits an AIO.
    #[cfg(aio)]
    Aio(NonNull<libc::aiocb>),
}

/// Low-level driver of polling.
pub(crate) struct Driver {
    events: Events,
    notify: Arc<Notify>,
    registry: HashMap<RawFd, FdQueue>,
    pool: AsyncifyPool,
    pool_completed: Arc<SegQueue<Entry>>,
}

impl Driver {
    pub fn new(builder: &ProactorBuilder) -> io::Result<Self> {
        instrument!(compio_log::Level::TRACE, "new", ?builder);
        trace!("new poll driver");

        let events = if let Some(cap) = NonZeroUsize::new(builder.capacity as _) {
            Events::with_capacity(cap)
        } else {
            Events::new()
        };
        let poll = Poller::new()?;
        let notify = Arc::new(Notify::new(poll));

        Ok(Self {
            events,
            notify,
            registry: HashMap::new(),
            pool: builder.create_or_get_thread_pool(),
            pool_completed: Arc::new(SegQueue::new()),
        })
    }

    pub fn driver_type(&self) -> DriverType {
        DriverType::Poll
    }

    pub fn create_key<T: crate::sys::OpCode + 'static>(&self, op: T) -> Key<T> {
        Key::new(self.as_raw_fd(), op)
    }

    fn poller(&self) -> &Poller {
        &self.notify.poll
    }

    fn with_events<F, R>(&mut self, f: F) -> R
    where
        F: FnOnce(&mut Self, &mut Events) -> R,
    {
        let mut events = std::mem::take(&mut self.events);
        let res = f(self, &mut events);
        self.events = events;
        res
    }

    /// # Safety
    /// The input fd should be valid.
    unsafe fn submit(&mut self, key: ErasedKey, arg: WaitArg) -> io::Result<()> {
        let need_add = !self.registry.contains_key(&arg.fd);
        let queue = self.registry.entry(arg.fd).or_default();
        queue.push_back_interest(key.into_raw(), arg.interest);
        let event = queue.event();
        if need_add {
            // SAFETY: the events are deleted correctly.
            unsafe { self.poller().add(arg.fd, event)? }
        } else {
            let fd = unsafe { BorrowedFd::borrow_raw(arg.fd) };
            self.poller().modify(fd, event)?;
        }
        Ok(())
    }

    fn renew(&mut self, fd: BorrowedFd, renew_event: Event) -> io::Result<()> {
        if !renew_event.readable && !renew_event.writable {
            self.poller().delete(fd)?;
            self.registry.remove(&fd.as_raw_fd());
        } else {
            self.poller().modify(fd, renew_event)?;
        }
        Ok(())
    }

    pub fn attach(&mut self, _fd: RawFd) -> io::Result<()> {
        Ok(())
    }

    pub fn cancel<T>(&mut self, key: Key<T>) {
        let op_type = key.borrow().pinned_op().op_type();
        match op_type {
            None => {}
            Some(OpType::Fd(fd)) => {
                let queue = self
                    .registry
                    .get_mut(&fd)
                    .expect("the fd should be attached");
                queue.remove(key.as_user_data());

                let renew_event = queue.event();
                let fd = unsafe { BorrowedFd::borrow_raw(fd) };
                if self.renew(fd, renew_event).is_ok() {
                    self.pool_completed.push(Entry::new_cancelled(key.erase()));
                }
            }
            #[cfg(aio)]
            Some(OpType::Aio(aiocbp)) => {
                let aiocb = unsafe { aiocbp.as_ref() };
                let fd = aiocb.aio_fildes;
                syscall!(libc::aio_cancel(fd, aiocbp.as_ptr())).ok();
            }
        }
    }

    pub fn push(&mut self, key: ErasedKey) -> Poll<io::Result<usize>> {
        instrument!(compio_log::Level::TRACE, "push", ?key);
        let decision = key.borrow().pinned_op().pre_submit()?;
        match decision {
            Decision::Wait(arg) => {
                // SAFETY: fd is from the OpCode.
                unsafe { self.submit(key, arg) }?;
                trace!("register {:?}", arg);
                Poll::Pending
            }
            Decision::Completed(res) => Poll::Ready(Ok(res)),
            Decision::Blocking => self.push_blocking(key),
            #[cfg(aio)]
            Decision::Aio(AioControl { mut aiocbp, submit }) => {
                let aiocb = unsafe { aiocbp.as_mut() };
                let user_data = key.as_user_data();
                #[cfg(freebsd)]
                {
                    // sigev_notify_kqueue
                    aiocb.aio_sigevent.sigev_signo = self.as_raw_fd();
                    aiocb.aio_sigevent.sigev_notify = libc::SIGEV_KEVENT;
                    aiocb.aio_sigevent.sigev_value.sival_ptr = user_data as _;
                }
                #[cfg(solarish)]
                let mut notify = libc::port_notify {
                    portnfy_port: self.as_raw_fd(),
                    portnfy_user: user_data as _,
                };
                #[cfg(solarish)]
                {
                    aiocb.aio_sigevent.sigev_notify = libc::SIGEV_PORT;
                    aiocb.aio_sigevent.sigev_value.sival_ptr = &mut notify as *mut _ as _;
                }
                match syscall!(submit(aiocbp.as_ptr())) {
                    Ok(_) => {
                        // Key is successfully submitted, leak it on this side.
                        key.into_raw();
                        Poll::Pending
                    }
                    // FreeBSD:
                    //   * EOPNOTSUPP: It's on a filesystem without AIO support. Just fallback to
                    //     blocking IO.
                    //   * EAGAIN: The process-wide queue is full. No safe way to remove the (maybe)
                    //     dead entries.
                    // Solarish:
                    //   * EAGAIN: Allocation failed.
                    Err(e)
                        if matches!(
                            e.raw_os_error(),
                            Some(libc::EOPNOTSUPP) | Some(libc::EAGAIN)
                        ) =>
                    {
                        self.push_blocking(key)
                    }
                    Err(e) => Poll::Ready(Err(e)),
                }
            }
        }
    }

    fn push_blocking(&mut self, key: ErasedKey) -> Poll<io::Result<usize>> {
        let waker = self.waker();
        let completed = self.pool_completed.clone();
        // SAFETY: we're submitting into the driver, so it's safe to freeze here.
        let mut key = unsafe { key.freeze() };

        let mut closure = move || {
            let poll = key.pinned_op().operate();
            let res = match poll {
                Poll::Pending => unreachable!("this operation is not non-blocking"),
                Poll::Ready(res) => res,
            };
            completed.push(Entry::new(key.into_inner(), res));
            waker.wake();
        };
        loop {
            match self.pool.dispatch(closure) {
                Ok(()) => return Poll::Pending,
                Err(e) => {
                    closure = e.0;
                    self.poll_blocking();
                }
            }
        }
    }

    fn poll_blocking(&mut self) -> bool {
        if self.pool_completed.is_empty() {
            return false;
        }
        while let Some(entry) = self.pool_completed.pop() {
            entry.notify();
        }
        true
    }

    pub fn poll(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        instrument!(compio_log::Level::TRACE, "poll", ?timeout);
        if self.poll_blocking() {
            return Ok(());
        }
        self.events.clear();
        self.notify.poll.wait(&mut self.events, timeout)?;
        if self.events.is_empty() && timeout.is_some() {
            return Err(io::Error::from_raw_os_error(libc::ETIMEDOUT));
        }
        self.with_events(|this, events| {
            for event in events.iter() {
                let user_data = event.key;
                trace!("receive {} for {:?}", user_data, event);
                // SAFETY: user_data is promised to be valid.
                let op = unsafe { BorrowedKey::from_raw(user_data) };
                let op_type = op.borrow().pinned_op().op_type();
                match op_type {
                    None => {
                        // On epoll, multiple event may be received even if it is registered as
                        // one-shot. It is safe to ignore it.
                        trace!("op {} is completed", user_data);
                    }
                    Some(OpType::Fd(fd)) => {
                        // If it's an FD op, the returned user_data is only for calling `op_type`.
                        // We need to pop the real user_data from the queue.
                        let queue = this
                            .registry
                            .get_mut(&fd)
                            .expect("the fd should be attached");
                        if let Some((user_data, interest)) = queue.pop_interest(&event) {
                            let poll = op.borrow().pinned_op().operate();

                            match poll {
                                // The operation should go back to the front.
                                Poll::Pending => queue.push_front_interest(user_data, interest),
                                Poll::Ready(res) => Entry::new(op.upgrade(), res).notify(),
                            };
                        }
                        let renew_event = queue.event();
                        let fd = unsafe { BorrowedFd::borrow_raw(fd) };
                        this.renew(fd, renew_event)?;
                    }
                    #[cfg(aio)]
                    Some(OpType::Aio(aiocbp)) => {
                        let err = unsafe { libc::aio_error(aiocbp.as_ptr()) };
                        let res = match err {
                            // If the user_data is reused but the previously registered event still
                            // emits (for example, HUP in epoll; however it is impossible now
                            // because we only use AIO on FreeBSD), we'd better ignore the current
                            // one and wait for the real event.
                            libc::EINPROGRESS => {
                                trace!("op {} is not completed", user_data);
                                continue;
                            }
                            libc::ECANCELED => {
                                // Remove the aiocb from kqueue.
                                unsafe { libc::aio_return(aiocbp.as_ptr()) };
                                Err(io::Error::from_raw_os_error(libc::ETIMEDOUT))
                            }
                            _ => {
                                syscall!(libc::aio_return(aiocbp.as_ptr())).map(|res| res as usize)
                            }
                        };
                        Entry::new(op.upgrade(), res).notify()
                    }
                }
            }

            Ok(())
        })
    }

    pub fn waker(&self) -> Waker {
        Waker::from(self.notify.clone())
    }

    pub fn create_buffer_pool(
        &mut self,
        buffer_len: u16,
        buffer_size: usize,
    ) -> io::Result<BufferPool> {
        #[cfg(fusion)]
        {
            Ok(BufferPool::new_poll(crate::FallbackBufferPool::new(
                buffer_len,
                buffer_size,
            )))
        }
        #[cfg(not(fusion))]
        {
            Ok(BufferPool::new(buffer_len, buffer_size))
        }
    }

    /// # Safety
    ///
    /// caller must make sure release the buffer pool with correct driver
    pub unsafe fn release_buffer_pool(&mut self, _: BufferPool) -> io::Result<()> {
        Ok(())
    }
}

impl AsRawFd for Driver {
    fn as_raw_fd(&self) -> RawFd {
        self.poller().as_raw_fd()
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        for fd in self.registry.keys() {
            unsafe {
                let fd = BorrowedFd::borrow_raw(*fd);
                self.poller().delete(fd).ok();
            }
        }
    }
}

impl Entry {
    pub(crate) fn new_cancelled(key: ErasedKey) -> Self {
        Entry::new(key, Err(io::Error::from_raw_os_error(libc::ETIMEDOUT)))
    }
}

/// A notify handle to the inner driver.
pub(crate) struct Notify {
    poll: Poller,
}

impl Notify {
    fn new(poll: Poller) -> Self {
        Self { poll }
    }

    /// Notify the inner driver.
    pub fn notify(&self) -> io::Result<()> {
        self.poll.notify()
    }
}

impl Wake for Notify {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.notify().ok();
    }
}
