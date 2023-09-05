use crate::driver::{Entry, Poller};
use crossbeam_queue::SegQueue;
use io_uring::{
    cqueue,
    opcode::MsgRingData,
    squeue,
    types::{Fd, SubmitArgs, Timespec},
    IoUring,
};
use std::{cell::RefCell, io, marker::PhantomData, mem::MaybeUninit, time::Duration};

pub(crate) use libc::{sockaddr_storage, socklen_t};
#[doc(no_inline)]
pub use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, RawFd};

pub(crate) mod fs;
pub(crate) mod net;
pub(crate) mod op;

/// Abstraction of io-uring operations.
pub trait OpCode {
    /// Create submission entry.
    fn create_entry(&mut self) -> squeue::Entry;
}

/// Low-level driver of io-uring.
pub struct Driver {
    inner: IoUring,
    squeue: SegQueue<squeue::Entry>,
    cqueue: SegQueue<Entry>,
    _p: PhantomData<RefCell<()>>,
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
            squeue: SegQueue::default(),
            cqueue: SegQueue::default(),
            _p: PhantomData,
        })
    }

    unsafe fn submit(&self, timeout: Option<Duration>) -> io::Result<()> {
        let mut inner_squeue = self.inner.submission_shared();
        // Anyway we need to submit once, no matter there are entries in squeue.
        loop {
            while !inner_squeue.is_full() {
                if let Some(entry) = self.squeue.pop() {
                    inner_squeue.push(&entry).unwrap();
                } else {
                    break;
                }
            }
            inner_squeue.sync();

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
            inner_squeue.sync();

            for entry in self.inner.completion_shared() {
                self.cqueue.push(create_entry(entry));
            }

            if self.squeue.is_empty() && inner_squeue.is_empty() {
                break;
            }
        }
        Ok(())
    }

    fn poll_entries(&self, entries: &mut [MaybeUninit<Entry>]) -> usize {
        let len = self.cqueue.len().min(entries.len());
        for entry in &mut entries[..len] {
            entry.write(self.cqueue.pop().unwrap());
        }
        len
    }
}

impl Poller for Driver {
    fn attach(&self, _fd: RawFd) -> io::Result<()> {
        Ok(())
    }

    unsafe fn push(&self, op: &mut impl OpCode, user_data: usize) -> io::Result<()> {
        let entry = op.create_entry().user_data(user_data as _);
        self.squeue.push(entry);
        Ok(())
    }

    fn post(&self, user_data: usize, result: usize) -> io::Result<()> {
        let entry = MsgRingData::new(
            Fd(self.inner.as_raw_fd()),
            result as i32,
            user_data as _,
            None,
        )
        .build();
        self.squeue.push(entry);
        Ok(())
    }

    fn poll(
        &self,
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
        unsafe { self.submit(timeout) }?;
        let len = self.poll_entries(entries);
        Ok(len)
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
