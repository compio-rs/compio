#[cfg_attr(all(doc, docsrs), doc(cfg(all())))]
#[allow(unused_imports)]
pub use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd};
use std::{
    io,
    os::fd::FromRawFd,
    pin::Pin,
    sync::Arc,
    task::{Poll, Wake, Waker},
    time::Duration,
};

use compio_log::{instrument, trace, warn};
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
use flume::{Receiver, Sender};
use io_uring::{
    IoUring,
    cqueue::more,
    opcode::{AsyncCancel, PollAdd},
    types::{Fd, SubmitArgs, Timespec},
};
use slab::Slab;

use crate::{
    AsyncifyPool, BufferPool, DriverType, Entry, ProactorBuilder,
    key::{ErasedKey, RefExt},
    syscall,
};

mod extra;
pub use extra::Extra;
pub(crate) mod op;

pub(crate) fn is_op_supported(code: u8) -> bool {
    #[cfg(feature = "once_cell_try")]
    use std::sync::OnceLock;

    #[cfg(not(feature = "once_cell_try"))]
    use once_cell::sync::OnceCell as OnceLock;

    static PROBE: OnceLock<io_uring::Probe> = OnceLock::new();

    PROBE
        .get_or_try_init(|| {
            let mut probe = io_uring::Probe::new();

            io_uring::IoUring::new(2)?
                .submitter()
                .register_probe(&mut probe)?;

            std::io::Result::Ok(probe)
        })
        .map(|probe| probe.is_supported(code))
        .unwrap_or_default()
}

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

impl OpEntry {
    fn personality(self, personality: Option<u16>) -> Self {
        let Some(personality) = personality else {
            return self;
        };

        match self {
            Self::Submission(entry) => Self::Submission(entry.personality(personality)),
            #[cfg(feature = "io-uring-sqe128")]
            Self::Submission128(entry) => Self::Submission128(entry.personality(personality)),
            Self::Blocking => Self::Blocking,
        }
    }
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
///
/// # Safety
///
/// The returned Entry from `create_entry` must be valid until the operation is
/// completed.
pub unsafe trait OpCode {
    /// Create submission entry.
    fn create_entry(self: Pin<&mut Self>) -> OpEntry;

    /// Create submission entry for fallback. This method will only be called if
    /// `create_entry` returns an entry with unsupported opcode.
    fn create_entry_fallback(self: Pin<&mut Self>) -> OpEntry {
        OpEntry::Blocking
    }

    /// Call the operation in a blocking way. This method will be called if
    /// * [`create_entry`] returns [`OpEntry::Blocking`].
    /// * [`create_entry`] returns an entry with unsupported opcode, and
    ///   [`create_entry_fallback`] returns [`OpEntry::Blocking`].
    /// * [`create_entry`] and [`create_entry_fallback`] both return an entry
    ///   with unsupported opcode.
    ///
    /// [`create_entry`]: OpCode::create_entry
    /// [`create_entry_fallback`]: OpCode::create_entry_fallback
    fn call_blocking(self: Pin<&mut Self>) -> io::Result<usize> {
        unreachable!("this operation is asynchronous")
    }

    /// Set the result when it successfully completes.
    /// The operation stores the result and is responsible to release it if the
    /// operation is cancelled.
    ///
    /// # Safety
    ///
    /// Users should not call it.
    unsafe fn set_result(self: Pin<&mut Self>, _: usize) {}
}

pub use OpCode as IourOpCode;

/// Low-level driver of io-uring.
pub(crate) struct Driver {
    inner: IoUring<SEntry, CEntry>,
    notifier: Notifier,
    pool: AsyncifyPool,
    completed_tx: Sender<Entry>,
    completed_rx: Receiver<Entry>,
    buffer_group_ids: Slab<()>,
    need_push_notifier: bool,
}

impl Driver {
    const CANCEL: u64 = u64::MAX;
    const NOTIFY: u64 = u64::MAX - 1;

    pub fn new(builder: &ProactorBuilder) -> io::Result<Self> {
        instrument!(compio_log::Level::TRACE, "new", ?builder);
        trace!("new iour driver");
        // if op_flags is empty, this loop will not run
        for code in builder.op_flags.get_codes() {
            if !is_op_supported(code) {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    format!("io-uring does not support opcode {code:?}({code})"),
                ));
            }
        }
        let notifier = Notifier::new()?;
        let mut io_uring_builder = IoUring::builder();
        if let Some(sqpoll_idle) = builder.sqpoll_idle {
            io_uring_builder.setup_sqpoll(sqpoll_idle.as_millis() as _);
        }
        if builder.coop_taskrun {
            io_uring_builder.setup_coop_taskrun();
        }
        if builder.taskrun_flag {
            io_uring_builder.setup_taskrun_flag();
        }

        let inner = io_uring_builder.build(builder.capacity)?;

        let submitter = inner.submitter();

        if let Some(fd) = builder.eventfd {
            submitter.register_eventfd(fd)?;
        }

        let (completed_tx, completed_rx) = flume::unbounded();

        Ok(Self {
            inner,
            notifier,
            completed_tx,
            completed_rx,
            pool: builder.create_or_get_thread_pool(),
            buffer_group_ids: Slab::new(),
            need_push_notifier: true,
        })
    }

    pub fn driver_type(&self) -> DriverType {
        DriverType::IoUring
    }

    #[allow(dead_code)]
    pub fn as_iour(&self) -> Option<&Self> {
        Some(self)
    }

    pub fn register_personality(&self) -> io::Result<u16> {
        self.inner.submitter().register_personality()
    }

    pub fn unregister_personality(&self, personality: u16) -> io::Result<()> {
        self.inner.submitter().unregister_personality(personality)
    }

    // Auto means that it choose to wait or not automatically.
    fn submit_auto(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        instrument!(compio_log::Level::TRACE, "submit_auto", ?timeout);

        // when taskrun is true, there are completed cqes wait to handle, no need to
        // block the submit
        let want_sqe = if self.inner.submission().taskrun() {
            0
        } else {
            1
        };

        let res = {
            // Last part of submission queue, wait till timeout.
            if let Some(duration) = timeout {
                let timespec = timespec(duration);
                let args = SubmitArgs::new().timespec(&timespec);
                self.inner.submitter().submit_with_args(want_sqe, &args)
            } else {
                self.inner.submit_and_wait(want_sqe)
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

    fn poll_blocking(&mut self) {
        while let Ok(entry) = self.completed_rx.try_recv() {
            entry.notify();
        }
    }

    fn poll_entries(&mut self) -> bool {
        self.poll_blocking();

        let mut cqueue = self.inner.completion();
        cqueue.sync();
        let has_entry = !cqueue.is_empty();
        for entry in cqueue {
            match entry.user_data() {
                Self::CANCEL => {}
                Self::NOTIFY => {
                    let flags = entry.flags();
                    if !more(flags) {
                        self.need_push_notifier = true;
                    }
                    self.notifier.clear().expect("cannot clear notifier");
                }
                _ => create_entry(entry).notify(),
            }
        }
        has_entry
    }

    pub fn default_extra(&self) -> Extra {
        Extra::new()
    }

    pub fn attach(&mut self, _fd: RawFd) -> io::Result<()> {
        Ok(())
    }

    pub fn cancel(&mut self, key: ErasedKey) {
        instrument!(compio_log::Level::TRACE, "cancel", ?key);
        trace!("cancel RawOp");
        unsafe {
            #[allow(clippy::useless_conversion)]
            if self
                .inner
                .submission()
                .push(
                    &AsyncCancel::new(key.as_raw() as _)
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

    fn push_raw_with_key(&mut self, entry: SEntry, key: ErasedKey) -> io::Result<()> {
        let entry = entry.user_data(key.as_raw() as _);
        self.push_raw(entry)?; // if push failed, do not leak the key. Drop it upon return.
        key.into_raw();
        Ok(())
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
                    self.poll_entries();
                    match self.submit_auto(Some(Duration::ZERO)) {
                        Ok(()) => {}
                        Err(e)
                            if matches!(
                                e.kind(),
                                io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
                            ) => {}
                        Err(e) => return Err(e),
                    }
                }
            }
        }
    }

    pub fn push(&mut self, key: ErasedKey) -> Poll<io::Result<usize>> {
        instrument!(compio_log::Level::TRACE, "push", ?key);
        let personality = key.borrow().extra().as_iour().get_personality();
        let mut op_entry = key
            .borrow()
            .pinned_op()
            .create_entry()
            .personality(personality);
        let mut fallbacked = false;
        trace!(?personality, "push Key");
        loop {
            match op_entry {
                OpEntry::Submission(entry) => {
                    if is_op_supported(entry.get_opcode() as _) {
                        #[allow(clippy::useless_conversion)]
                        self.push_raw_with_key(entry.into(), key)?;
                    } else if !fallbacked {
                        op_entry = key
                            .borrow()
                            .pinned_op()
                            .create_entry_fallback()
                            .personality(personality);
                        fallbacked = true;
                        continue;
                    } else {
                        self.push_blocking(key);
                    }
                }
                #[cfg(feature = "io-uring-sqe128")]
                OpEntry::Submission128(entry) => {
                    if is_op_supported(entry.get_opcode() as _) {
                        self.push_raw_with_key(entry, key)?;
                    } else if !fallbacked {
                        op_entry = key
                            .borrow()
                            .pinned_op()
                            .create_entry_fallback()
                            .personality(personality);
                        fallbacked = true;
                        continue;
                    } else {
                        self.push_blocking(key);
                    }
                }
                OpEntry::Blocking => self.push_blocking(key),
            }
            break;
        }
        Poll::Pending
    }

    fn push_blocking(&mut self, key: ErasedKey) {
        let waker = self.waker();
        let completed = self.completed_tx.clone();
        // SAFETY: we're submitting into the driver, so it's safe to freeze here.
        let mut key = unsafe { key.freeze() };
        let mut closure = move || {
            let res = key.pinned_op().call_blocking();
            let _ = completed.send(Entry::new(key.into_inner(), res));
            waker.wake();
        };
        while let Err(e) = self.pool.dispatch(closure) {
            closure = e.0;
            // do something to avoid busy loop
            self.poll_blocking();
            std::thread::yield_now();
        }
        self.poll_blocking();
    }

    pub fn poll(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        instrument!(compio_log::Level::TRACE, "poll", ?timeout);
        // Anyway we need to submit once, no matter if there are entries in squeue.
        trace!("start polling");

        if self.need_push_notifier {
            #[allow(clippy::useless_conversion)]
            self.push_raw(
                PollAdd::new(Fd(self.notifier.as_raw_fd()), libc::POLLIN as _)
                    .multi(true)
                    .build()
                    .user_data(Self::NOTIFY)
                    .into(),
            )?;
            self.need_push_notifier = false;
        }

        if !self.poll_entries() {
            self.submit_auto(timeout)?;
            self.poll_entries();
        }

        Ok(())
    }

    pub fn waker(&self) -> Waker {
        self.notifier.waker()
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
                io::ErrorKind::OutOfMemory,
                "too many buffer pool allocated",
            ));
        }

        let buf_ring = io_uring_buf_ring::IoUringBufRing::new_with_flags(
            &self.inner,
            buffer_len,
            buffer_group as _,
            buffer_size,
            0,
        )?;

        #[cfg(fusion)]
        {
            Ok(BufferPool::new_io_uring(crate::IoUringBufferPool::new(
                buf_ring,
            )))
        }
        #[cfg(not(fusion))]
        {
            Ok(BufferPool::new(buf_ring))
        }
    }

    /// # Safety
    ///
    /// caller must make sure release the buffer pool with correct driver
    pub unsafe fn release_buffer_pool(&mut self, buffer_pool: BufferPool) -> io::Result<()> {
        #[cfg(fusion)]
        let buffer_pool = buffer_pool.into_io_uring();

        let buffer_group = buffer_pool.buffer_group();
        unsafe { buffer_pool.into_inner().release(&self.inner)? };
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
    let key = unsafe { ErasedKey::from_raw(cq_entry.user_data() as _) };
    let mut entry = Entry::new(key, result);
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
    notify: Arc<Notify>,
}

impl Notifier {
    /// Create a new notifier.
    fn new() -> io::Result<Self> {
        let fd = syscall!(libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK))?;
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        Ok(Self {
            notify: Arc::new(Notify::new(fd)),
        })
    }

    pub fn clear(&self) -> io::Result<()> {
        loop {
            let mut buffer = [0u64];
            let res = syscall!(libc::read(
                self.as_raw_fd(),
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

    pub fn waker(&self) -> Waker {
        Waker::from(self.notify.clone())
    }
}

impl AsRawFd for Notifier {
    fn as_raw_fd(&self) -> RawFd {
        self.notify.fd.as_raw_fd()
    }
}

/// A notify handle to the inner driver.
#[derive(Debug)]
pub(crate) struct Notify {
    fd: OwnedFd,
}

impl Notify {
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

impl Wake for Notify {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.notify().ok();
    }
}
