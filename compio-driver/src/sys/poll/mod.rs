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
use smallvec::SmallVec;

use crate::{
    AsyncifyPool, BufferPool, DriverType, Entry, ErasedKey, ProactorBuilder,
    key::{BorrowedKey, RefExt},
    op::Interest,
    syscall,
};

mod extra;
pub use extra::Extra;
pub(crate) mod op;

struct Track {
    arg: WaitArg,
    ready: bool,
}

impl From<WaitArg> for Track {
    fn from(arg: WaitArg) -> Self {
        Self { arg, ready: false }
    }
}

/// Abstraction of operations.
///
/// # Safety
///
/// If `pre_submit` returns `Decision::Wait`, `op_type` must also return
/// `Some(OpType::Fd)` with same fds as the `WaitArg`s. Similarly, if
/// `pre_submit` returns `Decision::Aio`, `op_type` must return
/// `Some(OpType::Aio)` with the correct `aiocb` pointer.
pub unsafe trait OpCode {
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

/// One item in local or more items on heap.
type Multi<T> = SmallVec<[T; 1]>;

/// Result of [`OpCode::pre_submit`].
#[non_exhaustive]
pub enum Decision {
    /// Instant operation, no need to submit
    Completed(usize),
    /// Async operation, needs to submit
    Wait(Multi<WaitArg>),
    /// Blocking operation, needs to be spawned in another thread
    Blocking,
    /// AIO operation, needs to be spawned to the kernel.
    #[cfg(aio)]
    Aio(AioControl),
}

impl Decision {
    /// Decide to wait for the given fd with the given interest.
    pub fn wait_for(fd: RawFd, interest: Interest) -> Self {
        Self::Wait(SmallVec::from_buf([WaitArg { fd, interest }]))
    }

    /// Decide to wait for many fds.
    pub fn wait_for_many<I: IntoIterator<Item = WaitArg>>(args: I) -> Self {
        Self::Wait(Multi::from_iter(args))
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

impl WaitArg {
    /// Create a new readable `WaitArg`.
    pub fn readable(fd: RawFd) -> Self {
        Self {
            fd,
            interest: Interest::Readable,
        }
    }

    /// Create a new writable `WaitArg`.
    pub fn writable(fd: RawFd) -> Self {
        Self {
            fd,
            interest: Interest::Writable,
        }
    }
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
    read_queue: VecDeque<ErasedKey>,
    write_queue: VecDeque<ErasedKey>,
}

/// A token to remove an interest from `FdQueue`.
///
/// It is returned when an interest is pushed, and can be used to remove the
/// interest later. However do be careful that the index may be invalid or does
/// not correspond to the one inserted if other interests are added or removed
/// before it (toctou).
struct RemoveToken {
    idx: usize,
    is_read: bool,
}

impl RemoveToken {
    fn read(idx: usize) -> Self {
        Self { idx, is_read: true }
    }

    fn write(idx: usize) -> Self {
        Self {
            idx,
            is_read: false,
        }
    }
}

impl FdQueue {
    fn is_empty(&self) -> bool {
        self.read_queue.is_empty() && self.write_queue.is_empty()
    }

    fn remove_token(&mut self, token: RemoveToken) -> Option<ErasedKey> {
        if token.is_read {
            self.read_queue.remove(token.idx)
        } else {
            self.write_queue.remove(token.idx)
        }
    }

    pub fn push_back_interest(&mut self, key: ErasedKey, interest: Interest) -> RemoveToken {
        match interest {
            Interest::Readable => {
                self.read_queue.push_back(key);
                RemoveToken::read(self.read_queue.len() - 1)
            }
            Interest::Writable => {
                self.write_queue.push_back(key);
                RemoveToken::write(self.write_queue.len() - 1)
            }
        }
    }

    pub fn push_front_interest(&mut self, key: ErasedKey, interest: Interest) -> RemoveToken {
        let is_read = match interest {
            Interest::Readable => {
                self.read_queue.push_front(key);
                true
            }
            Interest::Writable => {
                self.write_queue.push_front(key);
                false
            }
        };
        RemoveToken { idx: 0, is_read }
    }

    pub fn remove(&mut self, key: &ErasedKey) {
        self.read_queue.retain(|k| k != key);
        self.write_queue.retain(|k| k != key);
    }

    pub fn event(&self) -> Event {
        let mut event = Event::none(0);
        if let Some(key) = self.read_queue.front() {
            event.readable = true;
            event.key = key.as_raw();
        }
        if let Some(key) = self.write_queue.front() {
            event.writable = true;
            event.key = key.as_raw();
        }
        event
    }

    pub fn pop_interest(&mut self, event: &Event) -> Option<(ErasedKey, Interest)> {
        if event.readable
            && let Some(key) = self.read_queue.pop_front()
        {
            return Some((key, Interest::Readable));
        }
        if event.writable
            && let Some(key) = self.write_queue.pop_front()
        {
            return Some((key, Interest::Writable));
        }
        None
    }
}

/// Represents the filter type of kqueue. `polling` crate doesn't expose such
/// API, and we need to know about it when `cancel` is called.
#[non_exhaustive]
pub enum OpType {
    /// The operation polls an fd.
    Fd(Multi<RawFd>),
    /// The operation submits an AIO.
    #[cfg(aio)]
    Aio(NonNull<libc::aiocb>),
}

impl OpType {
    /// Create an [`OpType::Fd`] with one [`RawFd`].
    pub fn fd(fd: RawFd) -> Self {
        Self::Fd(SmallVec::from_buf([fd]))
    }

    /// Create an [`OpType::Fd`] with multiple [`RawFd`]s.
    pub fn multi_fd<I: IntoIterator<Item = RawFd>>(fds: I) -> Self {
        Self::Fd(Multi::from_iter(fds))
    }
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

    pub fn default_extra(&self) -> Extra {
        Extra::new()
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

    fn try_get_queue(&mut self, fd: RawFd) -> Option<&mut FdQueue> {
        self.registry.get_mut(&fd)
    }

    fn get_queue(&mut self, fd: RawFd) -> &mut FdQueue {
        self.try_get_queue(fd).expect("the fd should be submitted")
    }

    /// Submit a new operation to the end of the queue.
    ///
    ///  # Safety
    /// The input fd should be valid.
    unsafe fn submit(&mut self, key: ErasedKey, arg: WaitArg) -> io::Result<()> {
        let Self {
            registry, notify, ..
        } = self;
        let need_add = !registry.contains_key(&arg.fd);
        let queue = registry.entry(arg.fd).or_default();
        let token = queue.push_back_interest(key, arg.interest);
        let event = queue.event();
        let res = if need_add {
            // SAFETY: the events are deleted correctly.
            unsafe { notify.poll.add(arg.fd, event) }
        } else {
            let fd = unsafe { BorrowedFd::borrow_raw(arg.fd) };
            notify.poll.modify(fd, event)
        };
        if res.is_err() {
            // Rollback the push if submission failed.
            queue.remove_token(token);
            if queue.is_empty() {
                registry.remove(&arg.fd);
            }
        }

        res
    }

    /// Submit a new operation to the front of the queue.
    ///
    /// # Safety
    /// The input fd should be valid.
    unsafe fn submit_front(&mut self, key: ErasedKey, arg: WaitArg) -> io::Result<()> {
        let need_add = !self.registry.contains_key(&arg.fd);
        let queue = self.registry.entry(arg.fd).or_default();
        queue.push_front_interest(key, arg.interest);
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

    /// Remove one interest from the queue.
    fn remove_one(&mut self, key: &ErasedKey, fd: RawFd) -> io::Result<()> {
        let Some(queue) = self.try_get_queue(fd) else {
            return Ok(());
        };
        queue.remove(key);
        let renew_event = queue.event();
        if queue.is_empty() {
            self.registry.remove(&fd);
        }
        self.renew(unsafe { BorrowedFd::borrow_raw(fd) }, renew_event)
    }

    /// Remove one interest from the queue, and emit a cancelled entry.
    fn cancel_one(&mut self, key: ErasedKey, fd: RawFd) -> Option<Entry> {
        self.remove_one(&key, fd)
            .map_or(None, |_| Some(Entry::new_cancelled(key)))
    }

    pub fn attach(&mut self, _fd: RawFd) -> io::Result<()> {
        Ok(())
    }

    pub fn cancel(&mut self, key: ErasedKey) {
        let op_type = key.borrow().pinned_op().op_type();
        match op_type {
            None => {}
            Some(OpType::Fd(fds)) => {
                let mut pushed = false;
                for fd in fds {
                    let entry = self.cancel_one(key.clone(), fd);
                    if !pushed && let Some(entry) = entry {
                        self.pool_completed.push(entry);
                        pushed = true;
                    }
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
        match { key.borrow().pinned_op().pre_submit()? } {
            Decision::Wait(args) => {
                key.borrow()
                    .extra_mut()
                    .as_poll_mut()
                    .set_args(args.clone());
                for arg in args.iter().copied() {
                    // SAFETY: fd is from the OpCode.
                    let res = unsafe { self.submit(key.clone(), arg) };
                    // if submission fails, remove all previously submitted fds.
                    if let Err(e) = res {
                        args.into_iter().for_each(|arg| {
                            // we don't care about renew errors
                            let _ = self.remove_one(&key, arg.fd);
                        });
                        return Poll::Ready(Err(e));
                    }
                    trace!("register {:?}", arg);
                }
                Poll::Pending
            }
            Decision::Completed(res) => Poll::Ready(Ok(res)),
            Decision::Blocking => self.push_blocking(key),
            #[cfg(aio)]
            Decision::Aio(AioControl { mut aiocbp, submit }) => {
                let aiocb = unsafe { aiocbp.as_mut() };
                let user_data = key.as_raw();
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

    #[allow(clippy::blocks_in_conditions)]
    fn poll_one(&mut self, event: Event, fd: RawFd) -> io::Result<()> {
        let queue = self.get_queue(fd);

        if let Some((key, _)) = queue.pop_interest(&event)
            && let mut op = key.borrow()
            && op.extra_mut().as_poll_mut().handle_event(fd)
        {
            // Add brace here to force `Ref` drop within the scrutinee
            match { op.pinned_op().operate() } {
                // Submit all fd's back to the front of the queue
                Poll::Pending => {
                    let extra = op.extra_mut().as_poll_mut();
                    extra.reset();
                    // `FdQueue` may have been removed, need to submit again
                    for t in extra.track.iter() {
                        let res = unsafe { self.submit_front(key.clone(), t.arg) };
                        if let Err(e) = res {
                            // On error, remove all previously submitted fds.
                            for t in extra.track.iter() {
                                let _ = self.remove_one(&key, t.arg.fd);
                            }
                            return Err(e);
                        }
                    }
                }
                Poll::Ready(res) => {
                    drop(op);
                    Entry::new(key, res).notify()
                }
            };
        }

        let renew_event = self.get_queue(fd).event();
        let fd = unsafe { BorrowedFd::borrow_raw(fd) };
        self.renew(fd, renew_event)
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
                trace!("receive {} for {:?}", event.key, event);
                // SAFETY: user_data is promised to be valid.
                let key = unsafe { BorrowedKey::from_raw(event.key) };
                let mut op = key.borrow();
                let op_type = op.pinned_op().op_type();
                match op_type {
                    None => {
                        // On epoll, multiple event may be received even if it is registered as
                        // one-shot. It is safe to ignore it.
                        trace!("op {} is completed", event.key);
                    }
                    Some(OpType::Fd(_)) => {
                        // FIXME: This should not happen
                        let Some(fd) = op.extra().as_poll().next_fd() else {
                            return Ok(());
                        };
                        drop(op);
                        this.poll_one(event, fd)?;
                    }
                    #[cfg(aio)]
                    Some(OpType::Aio(aiocbp)) => {
                        drop(op);
                        let err = unsafe { libc::aio_error(aiocbp.as_ptr()) };
                        let res = match err {
                            // If the user_data is reused but the previously registered event still
                            // emits (for example, HUP in epoll; however it is impossible now
                            // because we only use AIO on FreeBSD), we'd better ignore the current
                            // one and wait for the real event.
                            libc::EINPROGRESS => {
                                trace!("op {} is not completed", key.as_raw());
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
                        let key = unsafe { ErasedKey::from_raw(event.key) };
                        Entry::new(key, res).notify()
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
