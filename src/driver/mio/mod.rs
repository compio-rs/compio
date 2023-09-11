#[doc(no_inline)]
pub use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::{
    collections::{HashMap, VecDeque},
    io,
    mem::MaybeUninit,
    ops::ControlFlow,
    time::Duration,
};

use bitvec::prelude::{bitbox, BitBox, BitSlice};
pub(crate) use libc::{sockaddr_storage, socklen_t};
use mio::{
    event::{Event, Source},
    unix::SourceFd,
    Events, Interest, Poll, Token,
};

use crate::driver::{
    registered_fd::{FDRegistry, RegisteredFileAllocator, RegisteredFileDescriptors, UNREGISTERED},
    Entry, Poller,
};

pub(crate) mod op;

/// Abstraction of operations.
pub trait OpCode {
    /// Perform the operation before submit, and return [`Decision`] to
    /// indicate whether submitting the operation to mio is required.
    fn pre_submit(&mut self, fd_registry: &FilesRegistry) -> io::Result<Decision>;

    /// Perform the operation after received corresponding
    /// event.
    fn on_event(
        &mut self,
        event: &Event,
        fd_registry: &FilesRegistry,
    ) -> io::Result<ControlFlow<usize>>;
}

/// Result of [`OpCode::pre_submit`].
pub enum Decision {
    /// Instant operation, no need to submit
    Completed(usize),
    /// Async operation, needs to submit
    Wait(WaitArg),
}

impl Decision {
    /// Decide to wait for the given fd with the given interest.
    pub fn wait_for(fd: RawFd, interest: Interest) -> Self {
        Self::Wait(WaitArg { fd, interest })
    }

    /// Decide to wait for the given fd to be readable.
    pub fn wait_readable(fd: RawFd) -> Self {
        Self::wait_for(fd, Interest::READABLE)
    }

    /// Decide to wait for the given fd to be writable.
    pub fn wait_writable(fd: RawFd) -> Self {
        Self::wait_for(fd, Interest::WRITABLE)
    }
}

/// Meta of mio operations.
#[derive(Debug, Clone, Copy)]
pub struct WaitArg {
    fd: RawFd,
    interest: Interest,
}

/// Holds registered file descriptors
pub struct FilesRegistry {
    registered_files: Box<[RawFd]>,
}

/// Low-level driver of mio.
pub struct Driver {
    squeue: VecDeque<MioEntry>,
    cqueue: VecDeque<Entry>,
    events: Events,
    poll: Poll,
    waiting: HashMap<u64, WaitEntry>,
    registered_fd_bits: BitBox,
    registered_fd_search_from: u32,
    files_registry: FilesRegistry,
}

/// Entry in squeue
#[derive(Debug)]
struct MioEntry {
    op: *mut dyn OpCode,
    user_data: u64,
}

impl MioEntry {
    /// Safety: Caller mut guarantee that the op will live until it is
    /// completed.
    unsafe fn new(op: &mut (impl OpCode + 'static), user_data: u64) -> Self {
        Self {
            op: op as *mut dyn OpCode,
            user_data,
        }
    }

    fn op_mut(&mut self) -> &mut dyn OpCode {
        unsafe { &mut *self.op }
    }
}

/// Entry waiting for events
struct WaitEntry {
    op: *mut dyn OpCode,
    arg: WaitArg,
    user_data: u64,
}

impl WaitEntry {
    fn new(mio_entry: MioEntry, arg: WaitArg) -> Self {
        Self {
            op: mio_entry.op,
            arg,
            user_data: mio_entry.user_data,
        }
    }

    fn op_mut(&mut self) -> &mut dyn OpCode {
        unsafe { &mut *self.op }
    }
}

impl Driver {
    const DEFAULT_CAPACITY: u32 = 1024;

    /// Create a new mio driver with 1024 entries.
    pub fn new() -> io::Result<Self> {
        Self::with_entries(Self::DEFAULT_CAPACITY, Self::DEFAULT_CAPACITY)
    }

    /// Create a new mio driver with the given number of entries.
    pub fn with_entries(entries: u32, files_to_register: u32) -> io::Result<Self> {
        let entries = entries as usize; // for the sake of consistency, use u32 like iour
        let files_to_register = files_to_register as usize;

        Ok(Self {
            squeue: VecDeque::with_capacity(entries),
            cqueue: VecDeque::with_capacity(entries),
            events: Events::with_capacity(entries),
            poll: Poll::new()?,
            waiting: HashMap::new(),
            registered_fd_bits: bitbox![0; files_to_register],
            registered_fd_search_from: 0,
            files_registry: FilesRegistry {
                registered_files: vec![UNREGISTERED; files_to_register].into_boxed_slice(),
            },
        })
    }
}

impl Driver {
    fn submit(&mut self, entry: MioEntry, arg: WaitArg) -> io::Result<()> {
        let token = Token(usize::try_from(entry.user_data).expect("in u64 range"));

        SourceFd(&arg.fd).register(self.poll.registry(), token, arg.interest)?;

        // Only insert the entry after it was registered successfully
        self.waiting
            .insert(entry.user_data, WaitEntry::new(entry, arg));

        Ok(())
    }

    /// Register all operations in the squeue to mio.
    fn submit_squeue(&mut self) -> io::Result<()> {
        while let Some(mut entry) = self.squeue.pop_front() {
            match entry.op_mut().pre_submit(&mut self.files_registry) {
                Ok(Decision::Wait(arg)) => {
                    self.submit(entry, arg)?;
                }
                Ok(Decision::Completed(res)) => {
                    self.cqueue.push_back(Entry::new(entry.user_data, Ok(res)));
                }
                Err(err) => {
                    self.cqueue.push_back(Entry::new(entry.user_data, Err(err)));
                }
            }
        }

        Ok(())
    }

    /// Poll all events from mio, call `perform` on op and push them into
    /// cqueue.
    fn poll_impl(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.poll.poll(&mut self.events, timeout)?;
        for event in &self.events {
            let token = event.token();
            let entry = self
                .waiting
                .get_mut(&(token.0 as u64))
                .expect("Unknown token returned by mio"); // XXX: Should this be silently ignored?
            match { entry.op_mut().on_event(event, &mut self.files_registry) } {
                Ok(ControlFlow::Continue(_)) => {
                    continue;
                }
                Ok(ControlFlow::Break(res)) => {
                    self.cqueue.push_back(Entry::new(entry.user_data, Ok(res)));
                }
                Err(err) => {
                    self.cqueue.push_back(Entry::new(entry.user_data, Err(err)));
                }
            }
            self.poll
                .registry()
                .deregister(&mut SourceFd(&entry.arg.fd))?;
            self.waiting.remove(&(token.0 as u64));
        }
        Ok(())
    }

    fn poll_completed(&mut self, entries: &mut [MaybeUninit<Entry>]) -> usize {
        let len = self.cqueue.len().min(entries.len());
        for entry in &mut entries[..len] {
            entry.write(self.cqueue.pop_front().unwrap());
        }
        len
    }
}

impl Poller for Driver {
    unsafe fn push(&mut self, op: &mut (impl OpCode + 'static), user_data: u64) -> io::Result<()> {
        self.squeue.push_back(MioEntry::new(op, user_data));
        Ok(())
    }

    fn cancel(&mut self, user_data: u64) {
        let Some(entry) = self.waiting.remove(&user_data) else {
            return;
        };
        self.poll
            .registry()
            .deregister(&mut SourceFd(&entry.arg.fd))
            .ok();
    }

    fn poll(
        &mut self,
        timeout: Option<Duration>,
        entries: &mut [MaybeUninit<Entry>],
    ) -> io::Result<usize> {
        self.submit_squeue()?;
        if entries.is_empty() {
            return Ok(0);
        }
        let len = self.poll_completed(entries);
        if len > 0 {
            return Ok(len);
        }
        self.poll_impl(timeout)?;
        Ok(self.poll_completed(entries))
    }
}

impl AsRawFd for Driver {
    fn as_raw_fd(&self) -> RawFd {
        self.poll.as_raw_fd()
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
        _ = <FilesRegistry as FDRegistry>::register_files_update(
            &mut self.files_registry,
            offset,
            fds,
        )?;
        <Self as RegisteredFileAllocator>::register_files_update(self, offset, fds)
    }
}

impl FDRegistry for FilesRegistry {
    fn registered_files(&self) -> &[RawFd] {
        &self.registered_files
    }

    fn registered_files_mut(&mut self) -> &mut [RawFd] {
        &mut self.registered_files
    }
}
