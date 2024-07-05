#[cfg_attr(all(doc, docsrs), doc(cfg(all())))]
#[allow(unused_imports)]
pub use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::{io, io::ErrorKind, os::fd::FromRawFd, pin::Pin, sync::Arc, task::Poll, time::Duration};

use compio_log::{instrument, trace, warn};
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
    cqueue::more,
    opcode::{AsyncCancel, PollAdd},
    types::{Fd, SubmitArgs, Timespec},
    IoUring,
};
use io_uring_buf_ring::IoUringBufRing;
pub(crate) use libc::{sockaddr_storage, socklen_t};
use slab::Slab;

use crate::{syscall, AsyncifyPool, Entry, Key, OutEntries, ProactorBuilder};

pub(crate) mod buffer_pool;
pub(crate) mod op;

use buffer_pool::BufferPool;

/// The created entry of [`OpCode`].
pub enum OpEntry {
    /// This operation creates an io-uring submission entry.
    Submission(io_uring::squeue::Entry),
    #[cfg(feature = "io-uring-sqe128")]
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

#[cfg(feature = "io-uring-sqe128")]
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
    notifier: Notifier,
    pool: AsyncifyPool,
    pool_completed: Arc<SegQueue<Entry>>,
    // buffer group id type is u16, we should reuse the buffer group id when BufferPool is drop
    buffer_group_ids: Slab<()>,
}

impl Driver {
    const CANCEL: u64 = u64::MAX;
    const NOTIFY: u64 = u64::MAX - 1;

    pub fn new(builder: &ProactorBuilder) -> io::Result<Self> {
        instrument!(compio_log::Level::TRACE, "new", ?builder);
        trace!("new iour driver");
        let notifier = Notifier::new()?;
        let mut io_uring_builder = IoUring::builder();
        if let Some(sqpoll_idle) = builder.sqpoll_idle {
            io_uring_builder.setup_sqpoll(sqpoll_idle.as_millis() as _);
        }
        let mut inner = io_uring_builder.build(builder.capacity)?;
        #[allow(clippy::useless_conversion)]
        unsafe {
            inner
                .submission()
                .push(
                    &PollAdd::new(Fd(notifier.as_raw_fd()), libc::POLLIN as _)
                        .multi(true)
                        .build()
                        .user_data(Self::NOTIFY)
                        .into(),
                )
                .expect("the squeue sould not be full");
        }
        Ok(Self {
            inner,
            notifier,
            pool: builder.create_or_get_thread_pool(),
            pool_completed: Arc::new(SegQueue::new()),
            buffer_group_ids: Default::default(),
        })
    }

    // Auto means that it choose to wait or not automatically.
    fn submit_auto(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        instrument!(compio_log::Level::TRACE, "submit_auto", ?timeout);
        let res = {
            // Last part of submission queue, wait till timeout.
            if let Some(duration) = timeout {
                let timespec = timespec(duration);
                let args = SubmitArgs::new().timespec(&timespec);
                self.inner.submitter().submit_with_args(1, &args)
            } else {
                self.inner.submit_and_wait(1)
            }
        };
        trace!("submit result: {res:?}");
        match res {
            Ok(_) => {
                if self.inner.completion().is_empty() {
                    Err(io::ErrorKind::TimedOut.into())
                } else {
                    Ok(())
                }
            }
            Err(e) => match e.raw_os_error() {
                Some(libc::ETIME) => Err(io::ErrorKind::TimedOut.into()),
                Some(libc::EBUSY) | Some(libc::EAGAIN) => Err(io::ErrorKind::Interrupted.into()),
                _ => Err(e),
            },
        }
    }

    fn poll_entries(&mut self, entries: &mut impl Extend<Entry>) -> bool {
        // Cheaper than pop.
        if !self.pool_completed.is_empty() {
            while let Some(entry) = self.pool_completed.pop() {
                entries.extend(Some(entry));
            }
        }

        let mut cqueue = self.inner.completion();
        cqueue.sync();
        let has_entry = !cqueue.is_empty();
        let completed_entries = cqueue.filter_map(|entry| match entry.user_data() {
            Self::CANCEL => None,
            Self::NOTIFY => {
                let flags = entry.flags();
                debug_assert!(more(flags));
                self.notifier.clear().expect("cannot clear notifier");
                None
            }
            _ => Some(create_entry(entry)),
        });
        entries.extend(completed_entries);
        has_entry
    }

    pub fn create_op<T: crate::sys::OpCode + 'static>(&self, op: T) -> Key<T> {
        Key::new(self.as_raw_fd(), op)
    }

    pub fn attach(&mut self, _fd: RawFd) -> io::Result<()> {
        Ok(())
    }

    pub fn cancel<T>(&mut self, op: Key<T>) {
        instrument!(compio_log::Level::TRACE, "cancel", ?op);
        trace!("cancel RawOp");
        unsafe {
            #[allow(clippy::useless_conversion)]
            if self
                .inner
                .submission()
                .push(
                    &AsyncCancel::new(op.user_data() as _)
                        .build()
                        .user_data(Self::CANCEL)
                        .into(),
                )
                .is_err()
            {
                warn!("could not push AsyncCancel entry");
            }
        }
    }

    fn push_raw(&mut self, entry: SEntry) -> io::Result<()> {
        loop {
            let mut squeue = self.inner.submission();
            match unsafe { squeue.push(&entry) } {
                Ok(()) => {
                    squeue.sync();
                    break Ok(());
                }
                Err(_) => {
                    drop(squeue);
                    self.inner.submit()?;
                }
            }
        }
    }

    pub fn push<T: crate::sys::OpCode + 'static>(
        &mut self,
        op: &mut Key<T>,
    ) -> Poll<io::Result<usize>> {
        instrument!(compio_log::Level::TRACE, "push", ?op);
        let user_data = op.user_data();
        let op_pin = op.as_op_pin();
        trace!("push RawOp");
        match op_pin.create_entry() {
            OpEntry::Submission(entry) => {
                #[allow(clippy::useless_conversion)]
                self.push_raw(entry.user_data(user_data as _).into())?;
                Poll::Pending
            }
            #[cfg(feature = "io-uring-sqe128")]
            OpEntry::Submission128(entry) => {
                self.push_raw(entry.user_data(user_data as _))?;
                Poll::Pending
            }
            OpEntry::Blocking => {
                if self.push_blocking(user_data)? {
                    Poll::Pending
                } else {
                    Poll::Ready(Err(io::Error::from_raw_os_error(libc::EBUSY)))
                }
            }
        }
    }

    fn push_blocking(&mut self, user_data: usize) -> io::Result<bool> {
        let handle = self.handle()?;
        let completed = self.pool_completed.clone();
        let is_ok = self
            .pool
            .dispatch(move || {
                let mut op = unsafe { Key::<dyn crate::sys::OpCode>::new_unchecked(user_data) };
                let op_pin = op.as_op_pin();
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
        // Anyway we need to submit once, no matter there are entries in squeue.
        trace!("start polling");

        if !self.poll_entries(&mut entries) {
            self.submit_auto(timeout)?;
            self.poll_entries(&mut entries);
        }

        Ok(())
    }

    pub fn handle(&self) -> io::Result<NotifyHandle> {
        self.notifier.handle()
    }

    pub fn create_buffer_pool(
        &mut self,
        buffer_len: u16,
        buffer_size: usize,
    ) -> io::Result<BufferPool> {
        let buffer_group = self.buffer_group_ids.insert(());
        if buffer_group > u16::MAX as usize {
            self.buffer_group_ids.remove(buffer_group);

            return Err(io::Error::new(
                ErrorKind::OutOfMemory,
                "too many buffer pool allocated",
            ));
        }

        let buf_ring =
            IoUringBufRing::new(&self.inner, buffer_len, buffer_group as _, buffer_size)?;

        Ok(BufferPool::new(buf_ring))
    }

    /// # Safety
    ///
    /// caller must make sure release the buffer pool with correct driver
    pub unsafe fn release_buffer_pool(&mut self, buffer_pool: BufferPool) -> io::Result<()> {
        let buffer_group = buffer_pool.buffer_group();
        buffer_pool.into_inner().release(&self.inner)?;
        self.buffer_group_ids.remove(buffer_group as _);

        Ok(())
    }
}

impl AsRawFd for Driver {
    fn as_raw_fd(&self) -> RawFd {
        self.inner.as_raw_fd()
    }
}

fn create_entry(cq_entry: CEntry) -> Entry {
    let result = cq_entry.result();
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
    let mut entry = Entry::new(cq_entry.user_data() as _, result);
    entry.set_flags(cq_entry.flags());

    entry
}

fn timespec(duration: std::time::Duration) -> Timespec {
    Timespec::new()
        .sec(duration.as_secs())
        .nsec(duration.subsec_nanos())
}

#[derive(Debug)]
struct Notifier {
    fd: OwnedFd,
}

impl Notifier {
    /// Create a new notifier.
    fn new() -> io::Result<Self> {
        let fd = syscall!(libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK))?;
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        Ok(Self { fd })
    }

    pub fn clear(&self) -> io::Result<()> {
        loop {
            let mut buffer = [0u64];
            let res = syscall!(libc::read(
                self.fd.as_raw_fd(),
                buffer.as_mut_ptr().cast(),
                std::mem::size_of::<u64>()
            ));
            match res {
                Ok(len) => {
                    debug_assert_eq!(len, std::mem::size_of::<u64>() as _);
                    break Ok(());
                }
                // Clear the next time:)
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break Ok(()),
                // Just like read_exact
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => break Err(e),
            }
        }
    }

    pub fn handle(&self) -> io::Result<NotifyHandle> {
        Ok(NotifyHandle::new(self.fd.try_clone()?))
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
