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
    task::Poll,
    time::Duration,
};

use compio_log::{instrument, trace};
use crossbeam_queue::SegQueue;
use polling::{Event, Events, Poller};

use crate::{AsyncifyPool, BufferPool, Entry, Key, ProactorBuilder, op::Interest, syscall};

pub(crate) mod op;

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
    poll: Arc<Poller>,
    registry: HashMap<RawFd, FdQueue>,
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
        let event = queue.event();
        if need_add {
            self.poll.add(arg.fd, event)?;
        } else {
            let fd = BorrowedFd::borrow_raw(arg.fd);
            self.poll.modify(fd, event)?;
        }
        Ok(())
    }

    fn renew(
        poll: &Poller,
        registry: &mut HashMap<RawFd, FdQueue>,
        fd: BorrowedFd,
        renew_event: Event,
    ) -> io::Result<()> {
        if !renew_event.readable && !renew_event.writable {
            poll.delete(fd)?;
            registry.remove(&fd.as_raw_fd());
        } else {
            poll.modify(fd, renew_event)?;
        }
        Ok(())
    }

    pub fn attach(&mut self, _fd: RawFd) -> io::Result<()> {
        Ok(())
    }

    pub fn cancel(&mut self, op: &mut Key<dyn crate::sys::OpCode>) {
        let op_pin = op.as_op_pin();
        match op_pin.op_type() {
            None => {}
            Some(OpType::Fd(fd)) => {
                let queue = self
                    .registry
                    .get_mut(&fd)
                    .expect("the fd should be attached");
                queue.remove(op.user_data());
                let renew_event = queue.event();
                if Self::renew(
                    &self.poll,
                    &mut self.registry,
                    unsafe { BorrowedFd::borrow_raw(fd) },
                    renew_event,
                )
                .is_ok()
                {
                    self.pool_completed.push(entry_cancelled(op.user_data()));
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

    pub fn push(&mut self, op: &mut Key<dyn crate::sys::OpCode>) -> Poll<io::Result<usize>> {
        instrument!(compio_log::Level::TRACE, "push", ?op);
        let user_data = op.user_data();
        let op_pin = op.as_op_pin();
        match op_pin.pre_submit()? {
            Decision::Wait(arg) => {
                // SAFETY: fd is from the OpCode.
                unsafe {
                    self.submit(user_data, arg)?;
                }
                trace!("register {:?}", arg);
                Poll::Pending
            }
            Decision::Completed(res) => Poll::Ready(Ok(res)),
            Decision::Blocking => self.push_blocking(user_data),
            #[cfg(aio)]
            Decision::Aio(AioControl { mut aiocbp, submit }) => {
                let aiocb = unsafe { aiocbp.as_mut() };
                #[cfg(freebsd)]
                {
                    // sigev_notify_kqueue
                    aiocb.aio_sigevent.sigev_signo = self.poll.as_raw_fd();
                    aiocb.aio_sigevent.sigev_notify = libc::SIGEV_KEVENT;
                    aiocb.aio_sigevent.sigev_value.sival_ptr = user_data as _;
                }
                #[cfg(solarish)]
                let mut notify = libc::port_notify {
                    portnfy_port: self.poll.as_raw_fd(),
                    portnfy_user: user_data as _,
                };
                #[cfg(solarish)]
                {
                    aiocb.aio_sigevent.sigev_notify = libc::SIGEV_PORT;
                    aiocb.aio_sigevent.sigev_value.sival_ptr = &mut notify as *mut _ as _;
                }
                match syscall!(submit(aiocbp.as_ptr())) {
                    Ok(_) => Poll::Pending,
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
                        self.push_blocking(user_data)
                    }
                    Err(e) => Poll::Ready(Err(e)),
                }
            }
        }
    }

    fn push_blocking(&mut self, user_data: usize) -> Poll<io::Result<usize>> {
        let poll = self.poll.clone();
        let completed = self.pool_completed.clone();
        let mut closure = move || {
            let mut op = unsafe { Key::<dyn crate::sys::OpCode>::new_unchecked(user_data) };
            let op_pin = op.as_op_pin();
            let res = match op_pin.operate() {
                Poll::Pending => unreachable!("this operation is not non-blocking"),
                Poll::Ready(res) => res,
            };
            completed.push(Entry::new(user_data, res));
            poll.notify().ok();
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
            unsafe {
                entry.notify();
            }
        }
        true
    }

    pub unsafe fn poll(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        instrument!(compio_log::Level::TRACE, "poll", ?timeout);
        if self.poll_blocking() {
            return Ok(());
        }
        self.events.clear();
        self.poll.wait(&mut self.events, timeout)?;
        if self.events.is_empty() && timeout.is_some() {
            return Err(io::Error::from_raw_os_error(libc::ETIMEDOUT));
        }
        for event in self.events.iter() {
            let user_data = event.key;
            trace!("receive {} for {:?}", user_data, event);
            let mut op = Key::<dyn crate::sys::OpCode>::new_unchecked(user_data);
            let op = op.as_op_pin();
            match op.op_type() {
                None => {
                    // On epoll, multiple event may be received even if it is registered as
                    // one-shot. It is safe to ignore it.
                    trace!("op {} is completed", user_data);
                }
                Some(OpType::Fd(fd)) => {
                    // If it's an FD op, the returned user_data is only for calling `op_type`. We
                    // need to pop the real user_data from the queue.
                    let queue = self
                        .registry
                        .get_mut(&fd)
                        .expect("the fd should be attached");
                    if let Some((user_data, interest)) = queue.pop_interest(&event) {
                        let mut op = Key::<dyn crate::sys::OpCode>::new_unchecked(user_data);
                        let op = op.as_op_pin();
                        let res = match op.operate() {
                            Poll::Pending => {
                                // The operation should go back to the front.
                                queue.push_front_interest(user_data, interest);
                                None
                            }
                            Poll::Ready(res) => Some(res),
                        };
                        if let Some(res) = res {
                            Entry::new(user_data, res).notify();
                        }
                    }
                    let renew_event = queue.event();
                    Self::renew(
                        &self.poll,
                        &mut self.registry,
                        BorrowedFd::borrow_raw(fd),
                        renew_event,
                    )?;
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
                            libc::aio_return(aiocbp.as_ptr());
                            Err(io::Error::from_raw_os_error(libc::ETIMEDOUT))
                        }
                        _ => syscall!(libc::aio_return(aiocbp.as_ptr())).map(|res| res as usize),
                    };
                    Entry::new(user_data, res).notify();
                }
            }
        }
        Ok(())
    }

    pub fn handle(&self) -> NotifyHandle {
        NotifyHandle::new(self.poll.clone())
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
