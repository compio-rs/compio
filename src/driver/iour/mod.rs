#[doc(no_inline)]
pub use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::{collections::VecDeque, io, mem::MaybeUninit, time::Duration};

use io_uring::{
    cqueue,
    opcode::AsyncCancel,
    squeue,
    types::{SubmitArgs, Timespec},
    IoUring,
};
pub(crate) use libc::{sockaddr_storage, socklen_t};

use crate::driver::{Entry, Poller};

pub(crate) mod op;

/// Abstraction of io-uring operations.
pub trait OpCode {
    /// Create submission entry.
    fn create_entry(&mut self) -> squeue::Entry;
}

/// Low-level driver of io-uring.
pub struct Driver {
    inner: IoUring,
    squeue: VecDeque<squeue::Entry>,
    cqueue: VecDeque<Entry>,
}

impl Driver {
    /// Create a new io-uring driver with 1024 entries.
    pub fn new() -> io::Result<Self> {
        Self::with_entries(1024)
    }

    /// Create a new io-uring driver with specified entries.
    pub fn with_entries(entries: u32) -> io::Result<Self> {
        Ok(Self {
            inner: IoUring::new(entries)?,
            squeue: VecDeque::with_capacity(entries as usize),
            cqueue: VecDeque::with_capacity(entries as usize),
        })
    }

    fn submit(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        // Anyway we need to submit once, no matter there are entries in squeue.
        loop {
            {
                let mut inner_squeue = self.inner.submission();
                while !inner_squeue.is_full() {
                    if let Some(entry) = self.squeue.pop_front() {
                        unsafe { inner_squeue.push(&entry) }.unwrap();
                    } else {
                        break;
                    }
                }
                inner_squeue.sync();
            }

            let res = if self.squeue.is_empty() {
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
                    Some(libc::EBUSY) => Ok(()),
                    _ => Err(e),
                },
            }?;

            for entry in self.inner.completion() {
                let entry = create_entry(entry);
                if entry.user_data() == u64::MAX as _ {
                    // This is a cancel operation.
                    continue;
                }
                if let Err(e) = &entry.result {
                    if e.raw_os_error() == Some(libc::ECANCELED) {
                        // This operation is cancelled.
                        continue;
                    }
                }
                self.cqueue.push_back(entry);
            }

            if self.squeue.is_empty() && self.inner.submission().is_empty() {
                break;
            }
        }
        Ok(())
    }

    fn poll_entries(&mut self, entries: &mut [MaybeUninit<Entry>]) -> usize {
        let len = self.cqueue.len().min(entries.len());
        for entry in &mut entries[..len] {
            entry.write(self.cqueue.pop_front().unwrap());
        }
        len
    }
}

impl Poller for Driver {
    fn attach(&mut self, _fd: RawFd) -> io::Result<()> {
        Ok(())
    }

    unsafe fn push(
        &mut self,
        op: &mut (impl OpCode + 'static),
        user_data: usize,
    ) -> io::Result<()> {
        let entry = op.create_entry().user_data(user_data as _);
        self.squeue.push_back(entry);
        Ok(())
    }

    fn cancel(&mut self, user_data: usize) {
        self.squeue
            .push_back(AsyncCancel::new(user_data as _).build().user_data(u64::MAX));
    }

    fn poll(
        &mut self,
        timeout: Option<Duration>,
        entries: &mut [MaybeUninit<Entry>],
    ) -> io::Result<usize> {
        if entries.is_empty() {
            return Ok(0);
        }
        let len = self.poll_entries(entries);
        if len > 0 {
            return Ok(len);
        }
        self.submit(timeout)?;
        let len = self.poll_entries(entries);
        Ok(len)
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
