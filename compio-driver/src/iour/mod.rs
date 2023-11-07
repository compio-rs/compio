#[cfg_attr(all(doc, docsrs), doc(cfg(all())))]
#[allow(unused_imports)]
pub use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::{collections::VecDeque, io, pin::Pin, task::Poll, time::Duration};

use compio_log::{instrument, trace};
use io_uring::{
    cqueue,
    opcode::AsyncCancel,
    squeue,
    types::{SubmitArgs, Timespec},
    IoUring,
};
pub(crate) use libc::{sockaddr_storage, socklen_t};
use slab::Slab;

use crate::{Entry, ProactorBuilder};

pub(crate) mod op;
pub(crate) use crate::unix::RawOp;

/// Abstraction of io-uring operations.
pub trait OpCode {
    /// Create submission entry.
    fn create_entry(self: Pin<&mut Self>) -> squeue::Entry;
}

/// Low-level driver of io-uring.
pub(crate) struct Driver {
    inner: IoUring,
    squeue: VecDeque<squeue::Entry>,
}

impl Driver {
    const CANCEL: u64 = u64::MAX;

    pub fn new(builder: &ProactorBuilder) -> io::Result<Self> {
        instrument!(compio_log::Level::TRACE, "new", ?builder);
        trace!("new iour driver");
        Ok(Self {
            inner: IoUring::new(builder.capacity)?,
            squeue: VecDeque::with_capacity(builder.capacity as usize),
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
            Ok(_) => Ok(()),
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
        let completed_entries =
            self.inner
                .completion()
                .filter_map(|entry| match entry.user_data() {
                    Self::CANCEL => None,
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
        self.squeue.push_back(
            AsyncCancel::new(user_data as _)
                .build()
                .user_data(Self::CANCEL),
        );
    }

    pub fn push(&mut self, user_data: usize, op: &mut RawOp) -> Poll<io::Result<usize>> {
        instrument!(compio_log::Level::TRACE, "push", user_data);
        let op = op.as_pin();
        trace!("push RawOp");
        self.squeue
            .push_back(op.create_entry().user_data(user_data as _));
        Poll::Pending
    }

    pub unsafe fn poll(
        &mut self,
        timeout: Option<Duration>,
        entries: &mut impl Extend<Entry>,
        _registry: &mut Slab<RawOp>,
    ) -> io::Result<()> {
        instrument!(compio_log::Level::TRACE, "poll", ?timeout);
        // Anyway we need to submit once, no matter there are entries in squeue.
        trace!("start polling");
        loop {
            let ended = self.flush_submissions();

            self.submit_auto(timeout, ended)?;

            self.poll_entries(entries);

            if ended {
                trace!("polling ended");
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
