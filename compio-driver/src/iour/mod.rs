#[cfg_attr(all(doc, docsrs), doc(cfg(all())))]
#[allow(unused_imports)]
pub use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::{
    collections::VecDeque, io, os::fd::OwnedFd, pin::Pin, ptr::NonNull, sync::Arc, task::Poll,
    time::Duration,
};

use compio_log::{instrument, trace};
use crossbeam_queue::SegQueue;
cfg_if::cfg_if! {
    if #[cfg(feature = "io-uring-cqe32")] {
        use io_uring::cqueue::Entry32 as CEntry;
    } else {
        use io_uring::cqueue::Entry as CEntry;
    }
}
cfg_if::cfg_if! {
    if #[cfg(feature = "io-uring-sqe128")] {
        use io_uring::squeue::Entry128 as SEntry;
    } else {
        use io_uring::squeue::Entry as SEntry;
    }
}
use io_uring::{
    opcode::{AsyncCancel, Read},
    types::{Fd, SubmitArgs, Timespec},
    IoUring,
};
pub(crate) use libc::{sockaddr_storage, socklen_t};
use slab::Slab;

use crate::{syscall, AsyncifyPool, Entry, OutEntries, ProactorBuilder};

pub(crate) mod op;
pub(crate) use crate::unix::RawOp;

/// The created entry of [`OpCode`].
pub enum OpEntry {
    /// This operation creates an io-uring submission entry.
    Submission(io_uring::squeue::Entry),
    /// This operation creates an 128-bit io-uring submission entry.
    Submission128(io_uring::squeue::Entry128),
    /// This operation is a blocking one.
    Blocking,
}

impl From<io_uring::squeue::Entry> for OpEntry {
    fn from(value: io_uring::squeue::Entry) -> Self {
        Self::Submission(value)
    }
}

impl From<io_uring::squeue::Entry128> for OpEntry {
    fn from(value: io_uring::squeue::Entry128) -> Self {
        Self::Submission128(value)
    }
}

/// Abstraction of io-uring operations.
pub trait OpCode {
    /// Create submission entry.
    fn create_entry(self: Pin<&mut Self>) -> OpEntry;

    /// Call the operation in a blocking way. This method will only be called if
    /// [`create_entry`] returns [`OpEntry::Blocking`].
    fn call_blocking(self: Pin<&mut Self>) -> io::Result<usize> {
        unreachable!("this operation is asynchronous")
    }
}

/// Low-level driver of io-uring.
pub(crate) struct Driver {
    inner: IoUring<SEntry, CEntry>,
    squeue: VecDeque<SEntry>,
    notifier: Notifier,
    notifier_registered: bool,
    pool: AsyncifyPool,
    pool_completed: Arc<SegQueue<Entry>>,
}

impl Driver {
    const CANCEL: u64 = u64::MAX;
    const NOTIFY: u64 = u64::MAX - 1;

    pub fn new(builder: &ProactorBuilder) -> io::Result<Self> {
        instrument!(compio_log::Level::TRACE, "new", ?builder);
        trace!("new iour driver");
        Ok(Self {
            inner: IoUring::builder().build(builder.capacity)?,
            squeue: VecDeque::with_capacity(builder.capacity as usize),
            notifier: Notifier::new()?,
            notifier_registered: false,
            pool: builder.create_or_get_thread_pool(),
            pool_completed: Arc::new(SegQueue::new()),
        })
    }

    // Auto means that it choose to wait or not automatically.
    fn submit_auto(&mut self, timeout: Option<Duration>, wait: bool) -> io::Result<()> {
        instrument!(compio_log::Level::TRACE, "submit_auto", ?timeout, wait);
        let res = if wait {
            // Last part of submission queue, wait till timeout.
            if let Some(duration) = timeout {
                let timespec = timespec(duration);
                let args = SubmitArgs::new().timespec(&timespec);
                self.inner.submitter().submit_with_args(1, &args)
            } else {
                self.inner.submit_and_wait(1)
            }
        } else {
            self.inner.submit()
        };
        trace!("submit result: {res:?}");
        match res {
            Ok(_) => {
                if self.inner.completion().is_empty() {
                    Err(io::Error::from_raw_os_error(libc::ETIMEDOUT))
                } else {
                    Ok(())
                }
            }
            Err(e) => match e.raw_os_error() {
                Some(libc::ETIME) => Err(io::Error::from_raw_os_error(libc::ETIMEDOUT)),
                Some(libc::EBUSY) | Some(libc::EAGAIN) => Ok(()),
                _ => Err(e),
            },
        }
    }

    fn flush_submissions(&mut self) -> bool {
        instrument!(compio_log::Level::TRACE, "flush_submissions");

        let mut ended_ops = false;

        let mut inner_squeue = self.inner.submission();

        while !inner_squeue.is_full() {
            if self.squeue.len() <= inner_squeue.capacity() - inner_squeue.len() {
                trace!("inner_squeue have enough space, flush all entries");
                let (s1, s2) = self.squeue.as_slices();
                unsafe {
                    inner_squeue
                        .push_multiple(s1)
                        .expect("queue has enough space");
                    inner_squeue
                        .push_multiple(s2)
                        .expect("queue has enough space");
                }
                self.squeue.clear();
                ended_ops = true;
                break;
            } else if let Some(entry) = self.squeue.pop_front() {
                trace!("inner_squeue have not enough space, flush an entry");
                unsafe { inner_squeue.push(&entry) }.expect("queue has enough space");
            } else {
                trace!("self.squeue is empty, skip");
                ended_ops = true;
                break;
            }
        }

        inner_squeue.sync();

        ended_ops
    }

    fn poll_entries(&mut self, entries: &mut impl Extend<Entry>) {
        while let Some(entry) = self.pool_completed.pop() {
            entries.extend(Some(entry));
        }

        let mut cqueue = self.inner.completion();
        cqueue.sync();
        let completed_entries = cqueue.filter_map(|entry| match entry.user_data() {
            Self::CANCEL => None,
            Self::NOTIFY => {
                self.notifier_registered = false;
                None
            }
            _ => Some(create_entry(entry)),
        });
        entries.extend(completed_entries);
    }

    pub fn attach(&mut self, _fd: RawFd) -> io::Result<()> {
        Ok(())
    }

    pub fn cancel(&mut self, user_data: usize, _registry: &mut Slab<RawOp>) {
        instrument!(compio_log::Level::TRACE, "cancel", user_data);
        trace!("cancel RawOp");
        #[allow(clippy::useless_conversion)]
        self.squeue.push_back(
            AsyncCancel::new(user_data as _)
                .build()
                .user_data(Self::CANCEL)
                .into(),
        );
    }

    pub fn push(&mut self, user_data: usize, op: &mut RawOp) -> Poll<io::Result<usize>> {
        instrument!(compio_log::Level::TRACE, "push", user_data);
        let op_pin = op.as_pin();
        trace!("push RawOp");
        match op_pin.create_entry() {
            OpEntry::Submission(entry) => {
                #[allow(clippy::useless_conversion)]
                self.squeue
                    .push_back(entry.user_data(user_data as _).into());
                Poll::Pending
            }
            OpEntry::Submission128(_entry) => {
                #[cfg(feature = "io-uring-sqe128")]
                {
                    self.squeue.push_back(_entry.user_data(user_data as _));
                    Poll::Pending
                }
                #[cfg(not(feature = "io-uring-sqe128"))]
                {
                    Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "submission entry 128 is not enabled",
                    )))
                }
            }
            OpEntry::Blocking => {
                if self.push_blocking(user_data, op)? {
                    Poll::Pending
                } else {
                    Poll::Ready(Err(io::Error::from_raw_os_error(libc::EBUSY)))
                }
            }
        }
    }

    fn push_blocking(&mut self, user_data: usize, op: &mut RawOp) -> io::Result<bool> {
        // Safety: the RawOp is not released before the operation returns.
        struct SendWrapper<T>(T);
        unsafe impl<T> Send for SendWrapper<T> {}

        let op = SendWrapper(NonNull::from(op));
        let handle = self.handle()?;
        let completed = self.pool_completed.clone();
        let is_ok = self
            .pool
            .dispatch(move || {
                #[allow(clippy::redundant_locals)]
                let mut op = op;
                let op = unsafe { op.0.as_mut() };
                let op_pin = op.as_pin();
                let res = op_pin.call_blocking();
                completed.push(Entry::new(user_data, res));
                handle.notify().ok();
            })
            .is_ok();
        Ok(is_ok)
    }

    pub unsafe fn poll(
        &mut self,
        timeout: Option<Duration>,
        mut entries: OutEntries<impl Extend<usize>>,
    ) -> io::Result<()> {
        instrument!(compio_log::Level::TRACE, "poll", ?timeout);
        if !self.notifier_registered {
            let fd = self.notifier.as_raw_fd();
            let dst = self.notifier.dst();
            #[allow(clippy::useless_conversion)]
            self.squeue.push_back(
                Read::new(Fd(fd), dst.as_mut_ptr(), dst.len() as _)
                    .build()
                    .user_data(Self::NOTIFY)
                    .into(),
            );
            trace!("registered notifier");
            self.notifier_registered = true
        }
        // Anyway we need to submit once, no matter there are entries in squeue.
        trace!("start polling");
        loop {
            let ended = self.flush_submissions();

            self.submit_auto(timeout, ended)?;

            self.poll_entries(&mut entries);

            if ended {
                trace!("polling ended");
                break;
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
        self.inner.as_raw_fd()
    }
}

fn create_entry(entry: CEntry) -> Entry {
    let result = entry.result();
    let result = if result < 0 {
        let result = if result == -libc::ECANCELED {
            libc::ETIMEDOUT
        } else {
            -result
        };
        Err(io::Error::from_raw_os_error(result))
    } else {
        Ok(result as _)
    };
    Entry::new(entry.user_data() as _, result)
}

fn timespec(duration: std::time::Duration) -> Timespec {
    Timespec::new()
        .sec(duration.as_secs())
        .nsec(duration.subsec_nanos())
}

#[derive(Debug)]
struct Notifier {
    fd: OwnedFd,
    read_dst: Box<[u8; 8]>,
}

impl Notifier {
    /// Create a new notifier.
    fn new() -> io::Result<Self> {
        let fd = syscall!(libc::eventfd(0, libc::EFD_CLOEXEC))?;
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        Ok(Self {
            fd,
            read_dst: Box::new([0; 8]),
        })
    }

    fn dst(&mut self) -> &mut [u8] {
        self.read_dst.as_mut_slice()
    }

    pub fn handle(&self) -> io::Result<NotifyHandle> {
        let fd = self.fd.try_clone()?;
        Ok(NotifyHandle::new(fd))
    }
}

impl AsRawFd for Notifier {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

/// A notify handle to the inner driver.
pub struct NotifyHandle {
    fd: OwnedFd,
}

impl NotifyHandle {
    pub(crate) fn new(fd: OwnedFd) -> Self {
        Self { fd }
    }

    /// Notify the inner driver.
    pub fn notify(&self) -> io::Result<()> {
        let data = 1u64;
        syscall!(libc::write(
            self.fd.as_raw_fd(),
            &data as *const _ as *const _,
            std::mem::size_of::<u64>(),
        ))?;
        Ok(())
    }
}
