#[cfg_attr(all(doc, docsrs), doc(cfg(all())))]
#[allow(unused_imports)]
pub use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io::{self, Read, Write},
    num::NonZeroUsize,
    os::fd::BorrowedFd,
    pin::Pin,
    ptr::NonNull,
    sync::Arc,
    task::Poll,
    time::Duration,
};

use compio_log::{info, instrument, trace};
use crossbeam_queue::SegQueue;
pub(crate) use libc::{sockaddr_storage, socklen_t};
use polling::{Event, Events, PollMode, Poller};
use slab::Slab;

use crate::{syscall, AsyncifyPool, Entry, OutEntries, ProactorBuilder};

pub(crate) mod op;

pub(crate) use crate::unix::RawOp;

/// Abstraction of operations.
pub trait OpCode {
    /// Perform the operation before submit, and return [`Decision`] to
    /// indicate whether submitting the operation to polling is required.
    fn pre_submit(self: Pin<&mut Self>) -> io::Result<Decision>;

    /// Perform the operation after received corresponding
    /// event. If this operation is blocking, the return value should be
    /// [`Poll::Ready`].
    fn on_event(self: Pin<&mut Self>) -> Poll<io::Result<usize>>;
}

/// Result of [`OpCode::pre_submit`].
pub enum Decision {
    /// Instant operation, no need to submit
    Completed(usize),
    /// Async operation, needs to submit
    Wait(WaitArg),
    /// Blocking operation, needs to be spawned in another thread
    Blocking,
    /// AIO operation, needs to be spawned to the kernel. It is only supported
    /// on FreeBSD.
    Aio(AioArg),
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
    /// AIO is only supported on FreeBSD.
    pub fn aio(
        cb: &mut libc::aiocb,
        submit: unsafe extern "C" fn(*mut libc::aiocb) -> i32,
    ) -> Self {
        Self::Aio(AioArg {
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
#[derive(Debug, Clone, Copy)]
pub struct AioArg {
    /// Pointer of the control block.
    pub aiocbp: NonNull<libc::aiocb>,
    /// The aio_* submit function.
    pub submit: unsafe extern "C" fn(*mut libc::aiocb) -> i32,
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

// Represents the filter type of kqueue. `polling` crate doesn't expose such
// API, and we need to know about it when `cancel` is called.
enum OpType {
    Fd(RawFd),
    Aio(NonNull<libc::aiocb>),
}

/// Low-level driver of polling.
pub(crate) struct Driver {
    events: Events,
    poll: Arc<Poller>,
    registry: HashMap<RawFd, FdQueue>,
    cancelled: HashSet<usize>,
    keymap: HashMap<usize, OpType>,
    notifier: Notifier,
    pool: AsyncifyPool,
    pool_completed: Arc<SegQueue<Entry>>,
}

impl Driver {
    const NOTIFY: usize = usize::MAX - 1;

    pub fn new(builder: &ProactorBuilder) -> io::Result<Self> {
        instrument!(compio_log::Level::TRACE, "new", ?builder);
        trace!("new poll driver");
        let entries = builder.capacity as usize; // for the sake of consistency, use u32 like iour
        let events = if entries == 0 {
            Events::new()
        } else {
            Events::with_capacity(NonZeroUsize::new(entries).unwrap())
        };

        let notifier = Notifier::new()?;
        let fd = notifier.reader_fd();

        let poll = Arc::new(Poller::new()?);
        // Attach the reader to poll.
        unsafe {
            poll.add_with_mode(fd, Event::new(Self::NOTIFY, true, false), PollMode::Level)?;
        }

        Ok(Self {
            events,
            poll,
            registry: HashMap::new(),
            cancelled: HashSet::new(),
            keymap: HashMap::new(),
            notifier,
            pool: builder.create_or_get_thread_pool(),
            pool_completed: Arc::new(SegQueue::new()),
        })
    }

    pub fn create_op<T: crate::sys::OpCode + 'static>(&self, user_data: usize, op: T) -> RawOp {
        RawOp::new(user_data, op)
    }

    /// # Safety
    /// The input fd should be valid.
    unsafe fn submit(&mut self, user_data: usize, arg: WaitArg) -> io::Result<()> {
        let need_add = !self.registry.contains_key(&arg.fd);
        let queue = self.registry.entry(arg.fd).or_default();
        queue.push_back_interest(user_data, arg.interest);
        let event = queue.event(user_data);
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

    pub fn cancel(&mut self, user_data: usize, _registry: &mut Slab<RawOp>) {
        self.cancelled.insert(user_data);
        if let Some(OpType::Aio(aiocbp)) = self.keymap.get(&user_data) {
            if let Some(aiocb) = unsafe { aiocbp.as_ptr().as_ref() } {
                let fd = aiocb.aio_fildes;
                syscall!(libc::aio_cancel(fd, aiocbp.as_ptr())).ok();
            }
        }
    }

    pub fn push(&mut self, user_data: usize, op: &mut RawOp) -> Poll<io::Result<usize>> {
        instrument!(compio_log::Level::TRACE, "push", user_data);
        if self.cancelled.remove(&user_data) {
            Poll::Ready(Err(io::Error::from_raw_os_error(libc::ETIMEDOUT)))
        } else {
            let op_pin = op.as_pin();
            match op_pin.pre_submit()? {
                Decision::Wait(arg) => {
                    // SAFETY: fd is from the OpCode.
                    unsafe {
                        self.submit(user_data, arg)?;
                    }
                    trace!("register {:?}", arg);
                    self.keymap.insert(user_data, OpType::Fd(arg.fd));
                    Poll::Pending
                }
                Decision::Completed(res) => Poll::Ready(Ok(res)),
                Decision::Blocking => {
                    if self.push_blocking(user_data, op) {
                        Poll::Pending
                    } else {
                        Poll::Ready(Err(io::Error::from_raw_os_error(libc::EBUSY)))
                    }
                }
                Decision::Aio(AioArg { aiocbp, submit }) => {
                    #[cfg(target_os = "freebsd")]
                    if let Some(aiocb) = unsafe { aiocbp.as_ptr().as_mut() } {
                        // sigev_notify_kqueue
                        aiocb.aio_sigevent.sigev_signo = self.poll.as_raw_fd();
                        aiocb.aio_sigevent.sigev_notify = libc::SIGEV_KEVENT;
                        aiocb.aio_sigevent.sigev_value.sival_ptr = user_data as _;
                    }
                    syscall!(submit(aiocbp.as_ptr()))?;
                    self.keymap.insert(user_data, OpType::Aio(aiocbp));
                    Poll::Pending
                }
            }
        }
    }

    fn push_blocking(&mut self, user_data: usize, op: &mut RawOp) -> bool {
        // Safety: the RawOp is not released before the operation returns.
        struct SendWrapper<T>(T);
        unsafe impl<T> Send for SendWrapper<T> {}

        let op = SendWrapper(NonNull::from(op));
        let poll = self.poll.clone();
        let completed = self.pool_completed.clone();
        self.pool
            .dispatch(move || {
                #[allow(clippy::redundant_locals)]
                let mut op = op;
                let op = unsafe { op.0.as_mut() };
                let op_pin = op.as_pin();
                let res = match op_pin.on_event() {
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
        instrument!(compio_log::Level::TRACE, "poll", ?timeout);
        self.poll.wait(&mut self.events, timeout)?;
        if self.events.is_empty() && self.pool_completed.is_empty() && timeout.is_some() {
            return Err(io::Error::from_raw_os_error(libc::ETIMEDOUT));
        }
        while let Some(entry) = self.pool_completed.pop() {
            entries.extend(Some(entry));
        }
        for event in self.events.iter() {
            let user_data = event.key;
            if user_data == Self::NOTIFY {
                self.notifier.clear()?;
                continue;
            }
            trace!("receive {} for {:?}", user_data, event);
            match self.keymap.remove(&user_data) {
                None => {
                    // On epoll, multiple event may be received even if it is registered as
                    // one-shot. It is safe to ignore it.
                    info!("op {} is completed", user_data);
                }
                Some(OpType::Fd(fd)) => {
                    let queue = self
                        .registry
                        .get_mut(&fd)
                        .expect("the fd should be attached");
                    if let Some((user_data, interest)) = queue.pop_interest(&event) {
                        if self.cancelled.remove(&user_data) {
                            entries.extend(Some(entry_cancelled(user_data)));
                        } else {
                            let op = entries.registry()[user_data].as_pin();
                            let res = match op.on_event() {
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
                    let renew_event = queue.event(user_data);
                    let fd = BorrowedFd::borrow_raw(fd);
                    self.poll.modify(fd, renew_event)?;
                }
                Some(OpType::Aio(aiocbp)) => {
                    let err = unsafe { libc::aio_error(aiocbp.as_ptr()) };
                    let res = match err {
                        0 => {
                            let res = syscall!(libc::aio_return(aiocbp.as_ptr()))?;
                            Ok(res as usize)
                        }
                        // If the user_data is reused but the former registered event still
                        // emits (for example, HUP in epoll; however it is impossible now
                        // because we only use AIO on FreeBSD), we'd better ignore the current
                        // one and wait for the real event.
                        libc::EINPROGRESS => {
                            info!("op {} is not completed", user_data);
                            continue;
                        }
                        libc::ECANCELED => Err(io::Error::from_raw_os_error(libc::ETIMEDOUT)),
                        _ => Err(io::Error::from_raw_os_error(err)),
                    };
                    entries.extend(Some(Entry::new(user_data, res)));
                }
            }
        }
        Ok(())
    }

    pub fn handle(&self) -> io::Result<NotifyHandle> {
        self.notifier.handle()
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

struct Notifier {
    notify_reader: os_pipe::PipeReader,
    notify_writer: os_pipe::PipeWriter,
}

impl Notifier {
    pub fn new() -> io::Result<Self> {
        let (notify_reader, notify_writer) = os_pipe::pipe()?;

        // Set the reader as nonblocking.
        let fd = notify_reader.as_raw_fd();
        let current_flags = syscall!(libc::fcntl(fd, libc::F_GETFL))?;
        let flags = current_flags | libc::O_NONBLOCK;
        if flags != current_flags {
            syscall!(libc::fcntl(fd, libc::F_SETFL, flags))?;
        }

        Ok(Self {
            notify_reader,
            notify_writer,
        })
    }

    pub fn handle(&self) -> io::Result<NotifyHandle> {
        Ok(NotifyHandle::new(self.notify_writer.try_clone()?))
    }

    pub fn clear(&self) -> io::Result<()> {
        let mut buffer = [0u8];
        match (&self.notify_reader).read_exact(&mut buffer) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(()),
            Err(e) => Err(e),
        }
    }

    pub fn reader_fd(&self) -> RawFd {
        self.notify_reader.as_raw_fd()
    }
}

/// A notify handle to the inner driver.
pub struct NotifyHandle {
    sender: os_pipe::PipeWriter,
}

impl NotifyHandle {
    fn new(sender: os_pipe::PipeWriter) -> Self {
        Self { sender }
    }

    /// Notify the inner driver.
    pub fn notify(&self) -> io::Result<()> {
        (&self.sender).write_all(&[1u8])
    }
}
