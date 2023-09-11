#[doc(no_inline)]
pub use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::{collections::VecDeque, io, mem::MaybeUninit, time::Duration};

use bitvec::prelude::{bitbox, BitBox, BitSlice};
use io_uring::{
    cqueue,
    opcode::{self, AsyncCancel},
    squeue,
    types::{SubmitArgs, Timespec},
    IoUring, Probe,
};
pub(crate) use libc::{sockaddr_storage, socklen_t};

use super::{
    registered_fd::{RegisteredFileAllocator, UNREGISTERED},
    RegisteredFileDescriptors,
};
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
    registered_fd_bits: BitBox,
    registered_fd_search_from: u32,
}

impl Driver {
    /// Create a new io-uring driver with 1024 entries.
    pub fn new() -> io::Result<Self> {
        Self::with(1024, 1024)
    }

    /// Create a new io-uring driver with specified entries.
    pub fn with(entries: u32, files_to_register: u32) -> io::Result<Self> {
        let inner = IoUring::new(entries)?;
        let submitter = inner.submitter();
        let mut probe = Probe::new();
        submitter.register_probe(&mut probe)?;
        if probe.is_supported(opcode::Socket::CODE) {
            // register_files_sparse available since Linux 5.19
            submitter.register_files_sparse(files_to_register)?;
        } else {
            submitter.register_files(&vec![UNREGISTERED; files_to_register as usize])?;
        }
        Ok(Self {
            inner,
            squeue: VecDeque::with_capacity(entries as usize),
            cqueue: VecDeque::with_capacity(entries as usize),
            registered_fd_bits: bitbox![0; files_to_register as usize],
            registered_fd_search_from: 0,
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
                if entry.user_data() == u64::MAX {
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
    unsafe fn push(&mut self, op: &mut (impl OpCode + 'static), user_data: u64) -> io::Result<()> {
        let entry = op.create_entry().user_data(user_data);
        self.squeue.push_back(entry);
        Ok(())
    }

    fn cancel(&mut self, user_data: u64) {
        self.squeue
            .push_back(AsyncCancel::new(user_data).build().user_data(u64::MAX));
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

impl RegisteredFileAllocator for Driver {
    // bit slice of registered fds
    fn registered_bit_slice(&mut self) -> &BitSlice {
        self.registered_fd_bits.as_bitslice()
    }

    fn registered_bit_slice_mut(&mut self) -> &mut BitSlice {
        self.registered_fd_bits.as_mut_bitslice()
    }

    // where to start the next search for free registered fd
    fn registered_fd_search_from(&self) -> u32 {
        self.registered_fd_search_from
    }

    fn registered_fd_search_from_mut(&mut self) -> &mut u32 {
        &mut self.registered_fd_search_from
    }
}

impl RegisteredFileDescriptors for Driver {
    fn register_files_update(&mut self, offset: u32, fds: &[RawFd]) -> io::Result<usize> {
        let res = self.inner.submitter().register_files_update(offset, fds)?;
        _ = <Self as RegisteredFileAllocator>::register_files_update(self, offset, fds)?;
        Ok(res)
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
