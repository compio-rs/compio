#[cfg_attr(all(doc, docsrs), doc(cfg(all())))]
#[allow(unused_imports)]
pub use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd};
use std::{
    collections::HashSet,
    io,
    marker::PhantomData,
    os::fd::FromRawFd,
    sync::Arc,
    task::{Poll, Wake, Waker},
    time::Duration,
};

use compio_buf::BufResult;
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

use crate::{
    AsyncifyPool, DriverType, Entry, ProactorBuilder,
    control::Carrier,
    key::{BorrowedKey, ErasedKey},
    syscall,
};

mod buffer_pool;
mod extra;
pub(in crate::sys) use buffer_pool::BufControl;
pub(in crate::sys) use extra::Extra;
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
    pub(crate) fn with_extra(self, extra: &crate::Extra) -> Self {
        let Some(extra) = extra.try_as_iour() else {
            return self;
        };
        match self {
            Self::Submission(mut entry) => Self::Submission({
                if let Some(personality) = extra.get_personality() {
                    entry = entry.personality(personality);
                }
                // Set the union of two flags - it will not remove previous flags set by the Op
                entry.flags(extra.get_sqe_flags())
            }),
            #[cfg(feature = "io-uring-sqe128")]
            Self::Submission128(mut entry) => Self::Submission128({
                if let Some(personality) = extra.get_personality() {
                    entry = entry.personality(personality);
                }
                entry.flags(extra.get_sqe_flags())
            }),
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
    /// Type that contains self-references and other needed info during the
    /// operation
    type Control: Default;

    /// Initialize the control
    ///
    /// # Safety
    ///
    /// Caller must guarantee that during the lifetime of `ctrl`, `Self` is
    /// unmoved and valid.
    unsafe fn init(&mut self, ctrl: &mut Self::Control);

    /// Create submission entry.
    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry;

    /// Create submission entry for fallback. This method will only be called if
    /// `create_entry` returns an entry with unsupported opcode.
    fn create_entry_fallback(&mut self, _: &mut Self::Control) -> OpEntry {
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
    fn call_blocking(&mut self, _: &mut Self::Control) -> io::Result<usize> {
        unreachable!("this operation is asynchronous")
    }

    /// Set the result when it completes.
    /// The operation stores the result and is responsible to release it if the
    /// operation is cancelled.
    ///
    /// # Safety
    ///
    /// The params must be the result coming from this operation.
    unsafe fn set_result(
        &mut self,
        _: &mut Self::Control,
        _: &io::Result<usize>,
        _: &crate::Extra,
    ) {
    }

    /// Push a multishot result to the inner queue.
    ///
    /// # Safety
    ///
    /// The params must be the result coming from this operation.
    unsafe fn push_multishot(
        &mut self,
        _: &mut Self::Control,
        _: io::Result<usize>,
        _: crate::Extra,
    ) {
        unreachable!("this operation is not multishot")
    }

    /// Pop a multishot result from the inner queue.
    fn pop_multishot(
        &mut self,
        _: &mut Self::Control,
    ) -> Option<BufResult<usize, crate::sys::Extra>> {
        unreachable!("this operation is not multishot")
    }
}

pub(crate) trait Carry {
    /// See [`OpCode::create_entry`].
    fn create_entry(&mut self) -> OpEntry;

    /// See [`OpCode::create_entry_fallback`].
    fn create_entry_fallback(&mut self) -> OpEntry;

    /// See [`OpCode::call_blocking`].
    fn call_blocking(&mut self) -> io::Result<usize>;

    /// See [`OpCode::set_result`].
    unsafe fn set_result(&mut self, _: &io::Result<usize>, _: &crate::Extra);

    /// See [`OpCode::push_multishot`].
    unsafe fn push_multishot(&mut self, _: io::Result<usize>, _: crate::Extra);

    /// See [`OpCode::pop_multishot`].
    fn pop_multishot(&mut self) -> Option<BufResult<usize, crate::sys::Extra>>;
}

impl<T: crate::OpCode> Carry for Carrier<T> {
    fn create_entry(&mut self) -> OpEntry {
        let (op, control) = self.as_iour();
        op.create_entry(control)
    }

    fn create_entry_fallback(&mut self) -> OpEntry {
        let (op, control) = self.as_iour();
        op.create_entry_fallback(control)
    }

    fn call_blocking(&mut self) -> io::Result<usize> {
        let (op, control) = self.as_iour();
        op.call_blocking(control)
    }

    unsafe fn set_result(&mut self, result: &io::Result<usize>, extra: &crate::Extra) {
        let (op, control) = self.as_iour();
        unsafe { OpCode::set_result(op, control, result, extra) }
    }

    unsafe fn push_multishot(&mut self, result: io::Result<usize>, extra: crate::Extra) {
        let (op, control) = self.as_iour();
        unsafe { op.push_multishot(control, result, extra) }
    }

    fn pop_multishot(&mut self) -> Option<BufResult<usize, crate::sys::Extra>> {
        let (op, control) = self.as_iour();
        op.pop_multishot(control)
    }
}

pub use OpCode as IourOpCode;

/// Low-level driver of io-uring.
pub(crate) struct Driver {
    inner: IoUring<SEntry, CEntry>,
    notifier: Notifier,
    pool: AsyncifyPool,
    completed_tx: Sender<Entry>,
    completed_rx: Receiver<Entry>,
    need_push_notifier: bool,
    /// Keys leaked via `into_raw()` into io_uring user_data, freed on drop.
    in_flight: HashSet<usize>,
    _p: PhantomData<ErasedKey>,
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
        if let Some(cqsize) = builder.cqsize {
            io_uring_builder.setup_cqsize(cqsize);
        }
        io_uring_builder.dontfork();

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
            need_push_notifier: true,
            in_flight: HashSet::new(),
            _p: PhantomData,
        })
    }

    pub fn driver_type(&self) -> DriverType {
        DriverType::IoUring
    }

    #[allow(dead_code)]
    pub fn as_iour(&self) -> Option<&Self> {
        Some(self)
    }

    #[allow(dead_code)]
    pub fn as_iour_mut(&mut self) -> Option<&mut Self> {
        Some(self)
    }

    pub fn register_files(&self, fds: &[RawFd]) -> io::Result<()> {
        self.inner.submitter().register_files(fds)?;
        Ok(())
    }

    pub fn unregister_files(&self) -> io::Result<()> {
        self.inner.submitter().unregister_files()?;
        Ok(())
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

    fn poll_blocking(&mut self) -> bool {
        let mut has_entry = false;
        while let Ok(entry) = self.completed_rx.try_recv() {
            entry.notify();
            has_entry = true;
        }
        has_entry
    }

    fn poll_entries(&mut self) -> bool {
        let mut has_entry = self.poll_blocking();

        let mut cqueue = self.inner.completion();
        cqueue.sync();
        has_entry |= !cqueue.is_empty();
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
                key => {
                    let flags = entry.flags();
                    if more(flags) {
                        let key = unsafe { BorrowedKey::from_raw(key as _) };
                        let mut key = key.borrow();
                        let mut extra: crate::sys::Extra = Extra::new().into();
                        extra.set_flags(entry.flags());
                        unsafe {
                            key.carrier
                                .push_multishot(create_result(entry.result()), extra);
                        }
                        key.wake_by_ref();
                    } else {
                        self.in_flight.remove(&(key as usize));
                        create_entry(entry).notify()
                    }
                }
            }
        }
        has_entry
    }

    pub(in crate::sys) fn default_extra(&self) -> Extra {
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
        let user_data = key.as_raw();
        let entry = entry.user_data(user_data as _);
        self.push_raw(entry)?; // if push failed, do not leak the key. Drop it upon return.
        self.in_flight.insert(user_data);
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
        let mut op_entry = key.borrow().create_entry::<false>();
        let mut has_fallbacked = false;
        loop {
            trace!(fallback = has_fallbacked, "push entry");
            match op_entry {
                OpEntry::Submission(entry) => {
                    if is_op_supported(entry.get_opcode() as _) {
                        #[allow(clippy::useless_conversion)]
                        self.push_raw_with_key(entry.into(), key)?;
                    } else if !has_fallbacked {
                        op_entry = key.borrow().create_entry::<true>();
                        has_fallbacked = true;
                        continue;
                    } else {
                        self.push_blocking(key);
                    }
                }
                #[cfg(feature = "io-uring-sqe128")]
                OpEntry::Submission128(entry) => {
                    if is_op_supported(entry.get_opcode() as _) {
                        self.push_raw_with_key(entry, key)?;
                    } else if !has_fallbacked {
                        op_entry = key.borrow().create_entry::<true>();
                        has_fallbacked = true;
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
            let res = key.as_mut().carrier.call_blocking();
            let _ = completed.send(Entry::new(key.into_inner(), res));
            waker.wake();
        };
        while let Err(e) = self.pool.dispatch(closure) {
            closure = e.0;
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

    pub fn pop_multishot(
        &mut self,
        key: &ErasedKey,
    ) -> Option<BufResult<usize, crate::sys::Extra>> {
        key.borrow().carrier.pop_multishot()
    }
}

impl AsRawFd for Driver {
    fn as_raw_fd(&self) -> RawFd {
        self.inner.as_raw_fd()
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        // Drain completed CQEs first to avoid double-free.
        let mut cqueue = self.inner.completion();
        cqueue.sync();
        for entry in cqueue {
            match entry.user_data() {
                Self::CANCEL | Self::NOTIFY => {}
                key => {
                    self.in_flight.remove(&(key as usize));
                    drop(unsafe { ErasedKey::from_raw(key as _) });
                }
            }
        }

        // Free remaining in-flight keys.
        for user_data in self.in_flight.drain() {
            drop(unsafe { ErasedKey::from_raw(user_data) });
        }
    }
}

fn create_entry(cq_entry: CEntry) -> Entry {
    let result = cq_entry.result();
    let result = create_result(result);
    let key = unsafe { ErasedKey::from_raw(cq_entry.user_data() as _) };
    let mut entry = Entry::new(key, result);
    entry.set_flags(cq_entry.flags());

    entry
}

fn create_result(result: i32) -> io::Result<usize> {
    if result < 0 {
        Err(io::Error::from_raw_os_error(-result))
    } else {
        Ok(result as _)
    }
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
