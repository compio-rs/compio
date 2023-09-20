#[doc(no_inline)]
pub use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::{
    collections::{HashSet, VecDeque},
    io,
    pin::Pin,
    time::Duration,
};

use io_uring::{
    cqueue,
    opcode::AsyncCancel,
    squeue,
    types::{SubmitArgs, Timespec},
    IoUring,
};
pub(crate) use libc::{sockaddr_storage, socklen_t};
use slab::Slab;

use crate::driver::{Entry, RawOp};

pub(crate) mod op;

/// Abstraction of io-uring operations.
pub trait OpCode {
    /// Create submission entry.
    fn create_entry(self: Pin<&mut Self>) -> squeue::Entry;
}

/// Low-level driver of io-uring.
pub(crate) struct Driver {
    inner: IoUring,
    cancel_queue: VecDeque<u64>,
    cancelled: HashSet<u64>,
}

impl Driver {
    const CANCEL: u64 = u64::MAX;

    pub fn new(entries: u32) -> io::Result<Self> {
        Ok(Self {
            inner: IoUring::new(entries)?,
            cancel_queue: VecDeque::default(),
            cancelled: HashSet::default(),
        })
    }

    // Auto means that it choose to wait or not automatically.
    fn submit_auto(&mut self, timeout: Option<Duration>, wait: bool) -> io::Result<()> {
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
        match res {
            Ok(_) => Ok(()),
            Err(e) => match e.raw_os_error() {
                Some(libc::ETIME) => Err(io::Error::from_raw_os_error(libc::ETIMEDOUT)),
                Some(libc::EBUSY) | Some(libc::EAGAIN) => Ok(()),
                _ => Err(e),
            },
        }
    }

    fn flush_submissions(
        &mut self,
        ops: &mut impl Iterator<Item = usize>,
        registry: &mut Slab<RawOp>,
    ) -> bool {
        let mut ended_ops = false;
        let mut ended_cancel = false;

        let mut inner_squeue = self.inner.submission();

        while !inner_squeue.is_full() {
            if let Some(user_data) = ops.next() {
                let op = registry[user_data].as_pin();
                let entry = op.create_entry().user_data(user_data as _);
                unsafe { inner_squeue.push(&entry) }.expect("queue has enough space");
            } else {
                ended_ops = true;
                break;
            }
        }
        while !inner_squeue.is_full() {
            if let Some(user_data) = self.cancel_queue.pop_front() {
                let entry = AsyncCancel::new(user_data).build().user_data(Self::CANCEL);
                unsafe { inner_squeue.push(&entry) }.expect("queue has enough space");
            } else {
                ended_cancel = true;
                break;
            }
        }

        inner_squeue.sync();

        ended_ops && ended_cancel
    }

    fn poll_entries(&mut self, entries: &mut impl Extend<Entry>) {
        const SYSCALL_ECANCELED: i32 = -libc::ECANCELED;

        let completed_entries = self.inner.completion().filter_map(|entry| {
            if self.cancelled.remove(&entry.user_data()) {
                return None;
            }
            match (entry.user_data(), entry.result()) {
                (Self::CANCEL, _) | (_, SYSCALL_ECANCELED) => None,
                _ => Some(create_entry(entry)),
            }
        });
        entries.extend(completed_entries);
    }

    pub fn attach(&mut self, _fd: RawFd) -> io::Result<()> {
        Ok(())
    }

    pub fn cancel(&mut self, user_data: usize) {
        self.cancel_queue.push_back(user_data as _);
        self.cancelled.insert(user_data as _);
    }

    pub unsafe fn poll(
        &mut self,
        timeout: Option<Duration>,
        ops: &mut impl Iterator<Item = usize>,
        entries: &mut impl Extend<Entry>,
        registry: &mut Slab<RawOp>,
    ) -> io::Result<()> {
        let mut ops = ops.fuse();
        // Anyway we need to submit once, no matter there are entries in squeue.
        loop {
            let ended = self.flush_submissions(&mut ops, registry);

            self.submit_auto(timeout, ended)?;

            self.poll_entries(entries);

            if ended {
                break;
            }
        }
        Ok(())
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
        Err(io::Error::from_raw_os_error(-result))
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
