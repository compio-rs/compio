#[cfg(feature = "once_cell_try")]
use std::cell::OnceCell;
#[cfg_attr(all(doc, docsrs), doc(cfg(all())))]
#[allow(unused_imports)]
pub use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::{
    collections::{HashSet, VecDeque},
    io,
    os::fd::OwnedFd,
    pin::Pin,
    ptr::NonNull,
    sync::Arc,
    task::Poll,
    time::Duration,
};

use compio_log::{instrument, trace};
use crossbeam_queue::SegQueue;
use io_uring::{
    cqueue,
    opcode::{AsyncCancel, PollAdd, Read},
    squeue,
    types::{Fd, SubmitArgs, Timespec},
    CompletionQueue, IoUring,
};
pub(crate) use libc::{sockaddr_storage, socklen_t};
#[cfg(not(feature = "once_cell_try"))]
use once_cell::unsync::OnceCell;
use slab::Slab;

use crate::{syscall, AsyncifyPool, Entry, OutEntries, ProactorBuilder};

pub(crate) mod op;
pub(crate) use crate::unix::RawOp;

/// The created entry of [`OpCode`].
pub enum OpEntry {
    /// This operation creates an io-uring submission entry.
    Submission(squeue::Entry),
    /// This operation creates an 128-bit io-uring submission entry.
    Submission128(squeue::Entry128),
    /// This operation is a blocking one.
    Blocking,
}

impl From<squeue::Entry> for OpEntry {
    fn from(value: squeue::Entry) -> Self {
        Self::Submission(value)
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

struct UnboundedUring<S: squeue::EntryMarker, C: cqueue::EntryMarker> {
    inner: IoUring<S, C>,
    squeue: VecDeque<S>,
}

impl<S: squeue::EntryMarker, C: cqueue::EntryMarker> UnboundedUring<S, C> {
    pub fn new(capacity: u32) -> io::Result<Self> {
        Ok(Self {
            inner: IoUring::builder().build(capacity)?,
            squeue: VecDeque::with_capacity(capacity as _),
        })
    }

    pub fn submit(&mut self, timeout: Option<Duration>, wait: bool) -> io::Result<()> {
        instrument!(compio_log::Level::TRACE, "submit", ?timeout, wait);
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
                if wait && self.inner.completion().is_empty() {
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

    pub fn flush_submissions(&mut self) -> bool {
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

    pub fn completion(&mut self) -> CompletionQueue<C> {
        self.inner.completion()
    }

    pub fn push_submission(&mut self, entry: S) {
        self.squeue.push_back(entry)
    }
}

impl<S: squeue::EntryMarker, C: cqueue::EntryMarker> AsRawFd for UnboundedUring<S, C> {
    fn as_raw_fd(&self) -> RawFd {
        self.inner.as_raw_fd()
    }
}

/// Low-level driver of io-uring.
pub(crate) struct Driver {
    inner: UnboundedUring<squeue::Entry, cqueue::Entry>,
    inner_128: OnceCell<UnboundedUring<squeue::Entry128, cqueue::Entry32>>,
    capacity: u32,
    user_data_128: HashSet<usize>,
    notifier: Notifier,
    notifier_registered: bool,
    pool: AsyncifyPool,
    pool_completed: Arc<SegQueue<Entry>>,
}

impl Driver {
    const CANCEL: u64 = u64::MAX;
    const NOTIFY: u64 = u64::MAX - 1;
    const POLL128: u64 = u64::MAX - 2;

    pub fn new(builder: &ProactorBuilder) -> io::Result<Self> {
        instrument!(compio_log::Level::TRACE, "new", ?builder);
        trace!("new iour driver");
        Ok(Self {
            inner: UnboundedUring::new(builder.capacity)?,
            inner_128: OnceCell::new(),
            capacity: builder.capacity,
            user_data_128: HashSet::new(),
            notifier: Notifier::new()?,
            notifier_registered: false,
            pool: builder.create_or_get_thread_pool(),
            pool_completed: Arc::new(SegQueue::new()),
        })
    }

    fn inner_128(&mut self) -> io::Result<&mut UnboundedUring<squeue::Entry128, cqueue::Entry32>> {
        self.inner_128.get_or_try_init(|| {
            let ring = UnboundedUring::new(self.capacity)?;
            self.inner.push_submission(
                PollAdd::new(Fd(ring.as_raw_fd()), libc::POLLIN as _)
                    .multi(true)
                    .build()
                    .user_data(Self::POLL128),
            );
            io::Result::Ok(ring)
        })?;
        Ok(self.inner_128.get_mut().expect("inner_128 should be set"))
    }

    fn poll_entries(&mut self, entries: &mut impl Extend<Entry>) {
        while let Some(entry) = self.pool_completed.pop() {
            entries.extend(Some(entry));
        }

        let mut cqueue = self.inner.completion();
        cqueue.sync();
        let completed_entries = cqueue.filter_map(|entry| match entry.user_data() {
            Self::CANCEL | Self::POLL128 => None,
            Self::NOTIFY => {
                self.notifier_registered = false;
                None
            }
            _ => Some(create_entry(entry)),
        });
        entries.extend(completed_entries);

        // TODO: only poll it when POLL128 is triggered?
        if let Some(inner_128) = self.inner_128.get_mut() {
            let mut cqueue = inner_128.completion();
            cqueue.sync();
            let completed_entries = cqueue.filter_map(|entry| match entry.user_data() {
                Self::CANCEL => None,
                _ => {
                    self.user_data_128.remove(&(entry.user_data() as _));
                    Some(create_entry(entry.into()))
                }
            });
            entries.extend(completed_entries);
        }
    }

    pub fn attach(&mut self, _fd: RawFd) -> io::Result<()> {
        Ok(())
    }

    pub fn cancel(&mut self, user_data: usize, _registry: &mut Slab<RawOp>) {
        instrument!(compio_log::Level::TRACE, "cancel", user_data);
        trace!("cancel RawOp");
        let use_128 = self.inner_128.get().is_some() && self.user_data_128.contains(&user_data);
        if use_128 {
            self.inner_128
                .get_mut()
                .expect("inner_128 should be set")
                .push_submission(
                    AsyncCancel::new(user_data as _)
                        .build()
                        .user_data(Self::CANCEL)
                        .into(),
                );
        } else {
            self.inner.push_submission(
                AsyncCancel::new(user_data as _)
                    .build()
                    .user_data(Self::CANCEL),
            );
        }
    }

    pub fn push(&mut self, user_data: usize, op: &mut RawOp) -> Poll<io::Result<usize>> {
        instrument!(compio_log::Level::TRACE, "push", user_data);
        let op_pin = op.as_pin();
        trace!("push RawOp");
        match op_pin.create_entry() {
            OpEntry::Submission(entry) => {
                self.inner.push_submission(entry.user_data(user_data as _));
                Poll::Pending
            }
            OpEntry::Submission128(entry) => {
                self.user_data_128.insert(user_data);
                self.inner_128()?
                    .push_submission(entry.user_data(user_data as _));
                Poll::Pending
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
            self.inner.push_submission(
                Read::new(Fd(fd), dst.as_mut_ptr(), dst.len() as _)
                    .build()
                    .user_data(Self::NOTIFY),
            );
            trace!("registered notifier");
            self.notifier_registered = true
        }
        // If 128 uring is created, flush the submissions for it.
        if let Some(inner_128) = self.inner_128.get_mut() {
            trace!("push 128-bit entries");
            loop {
                let ended = inner_128.flush_submissions();
                // Don't wait for it. Poll it in the main ring.
                inner_128.submit(None, false)?;
                if ended {
                    break;
                }
            }
        }
        // Anyway we need to submit once, no matter there are entries in squeue.
        trace!("start polling");
        loop {
            let ended = self.inner.flush_submissions();

            self.inner.submit(timeout, ended)?;

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

fn create_entry(entry: cqueue::Entry) -> Entry {
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
