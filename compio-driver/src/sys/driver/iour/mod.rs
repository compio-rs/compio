use std::{
    collections::HashSet, marker::PhantomData, mem::ManuallyDrop, panic::AssertUnwindSafe,
    sync::Arc, time::Duration,
};

use crate::sys::{extra::IourExtra, prelude::*};

mod_use![op, notify];

cfg_select! {
    feature = "io-uring-cqe32" => {
        use io_uring::cqueue::Entry32 as CEntry;
    }
    _ => {
        use io_uring::cqueue::Entry as CEntry;
    }
}

cfg_select! {
    feature = "io-uring-sqe128" => {
        use io_uring::squeue::Entry128 as SEntry;
    }
    _ => {
        use io_uring::squeue::Entry as SEntry;
    }
}

use flume::{Receiver, Sender};
use io_uring::{
    EnterFlags, IoUring,
    cqueue::more,
    opcode::{AsyncCancel, PollAdd},
    types::{Fd, SubmitArgs, Timespec},
};

use crate::{
    AsyncifyPool, DriverType, Entry, ProactorBuilder,
    key::{BorrowedKey, ErasedKey},
    panic::catch_unwind_io,
};

bitflags::bitflags! {
    /// Mutable driver state tracked as a small bit set.
    #[derive(Clone, Copy)]
    struct DriverFlags: u8 {
        /// The multishot notifier `PollAdd` is no longer armed and must be
        /// re-pushed before the next wait.
        const NEED_PUSH_NOTIFIER = 1 << 0;
        /// Set `IORING_ENTER_NO_IOWAIT` on blocking waits so an idle ring is not
        /// charged as iowait. Enabled only when SQPOLL is unused and the kernel
        /// reports `IORING_FEAT_NO_IOWAIT` (since 6.15).
        ///
        /// See io_uring_enter(2):
        /// <https://man7.org/linux/man-pages/man2/io_uring_enter.2.html>
        const NO_IOWAIT = 1 << 1;
    }
}

/// Low-level driver of io-uring.
pub(crate) struct Driver {
    // Wrapped in `ManuallyDrop` so that `Drop` can close the ring *before*
    // releasing the in-flight keys. Closing the io_uring fd makes the kernel
    // wait for or cancel any in-flight ops, which guarantees the kernel is no
    // longer reading from or writing to the buffers owned by those keys.
    inner: ManuallyDrop<IoUring<SEntry, CEntry>>,
    notifier: Notifier,
    pool: AsyncifyPool,
    completed_tx: Sender<Entry>,
    completed_rx: Receiver<Entry>,
    flags: DriverFlags,
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
            if let Some(cpu) = builder.sqpoll_cpu {
                io_uring_builder.setup_sqpoll_cpu(cpu);
            }
        }
        if builder.single_issuer {
            io_uring_builder.setup_single_issuer();
            if builder.defer_taskrun {
                io_uring_builder.setup_defer_taskrun();
            }
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

        // NO_IOWAIT needs kernel 6.15+ (the IORING_FEAT_NO_IOWAIT feature bit)
        // and is meaningless under SQPOLL: the polling thread, not an `enter`
        // wait, drives submission, so there is no CQE wait to mark.
        let mut flags = DriverFlags::NEED_PUSH_NOTIFIER;
        flags.set(
            DriverFlags::NO_IOWAIT,
            builder.sqpoll_idle.is_none() && inner.params().is_feature_no_iowait(),
        );

        Ok(Self {
            inner: ManuallyDrop::new(inner),
            notifier,
            completed_tx,
            completed_rx,
            pool: builder.create_or_get_thread_pool(),
            flags,
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

    pub(in crate::sys) fn inner(&mut self) -> &mut IoUring<SEntry, CEntry> {
        &mut self.inner
    }

    // Auto means that it choose to wait or not automatically.
    fn submit_auto(&mut self, timeout: Option<Duration>, need_wait: bool) -> io::Result<()> {
        instrument!(compio_log::Level::TRACE, "submit_auto", ?timeout);

        // when taskrun is true, there are completed cqes wait to handle, no need to
        // block the submit
        let want_sqe = if !need_wait || self.inner.submission().taskrun() {
            0
        } else {
            1
        };

        // Only a wait that can actually sleep is charged as iowait; a zero
        // timeout (the drain calls from `push_raw`/`flush`) returns immediately,
        // so it keeps the plain combined path.
        //
        // On the sleeping path, opt out of iowait accounting (see
        // `DriverFlags::NO_IOWAIT`) by carrying NO_IOWAIT on the same
        // submit-and-wait `enter`.
        let can_block = want_sqe > 0 && timeout != Some(Duration::ZERO);
        let res = if self.flags.contains(DriverFlags::NO_IOWAIT) && can_block {
            self.submit_and_wait_no_iowait(want_sqe, timeout)
        } else {
            self.submit_and_wait(want_sqe, timeout)
        };
        trace!("submit result: {res:?}");
        match res {
            Ok(_) => {
                if want_sqe > 0 && self.inner.completion().is_empty() {
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

    /// The combined submit+wait. Used for zero-timeout drains and when
    /// `NO_IOWAIT` is unavailable.
    fn submit_and_wait(&self, want_sqe: usize, timeout: Option<Duration>) -> io::Result<usize> {
        if let Some(duration) = timeout {
            let timespec = timespec(duration);
            let args = SubmitArgs::new().timespec(&timespec);
            self.inner.submitter().submit_with_args(want_sqe, &args)
        } else {
            self.inner.submit_and_wait(want_sqe)
        }
    }

    /// Submit the pending SQEs and wait on completions in a single `enter`
    /// carrying `IORING_ENTER_NO_IOWAIT` so the wait is not charged as iowait
    /// (see `DriverFlags::NO_IOWAIT`). The crate's `submit_*` helpers cannot add
    /// custom `EnterFlags`, so this drops to the raw `enter` with
    /// `to_submit = sq_len()` instead of the combined `submit_and_wait`.
    fn submit_and_wait_no_iowait(
        &mut self,
        want_sqe: usize,
        timeout: Option<Duration>,
    ) -> io::Result<usize> {
        // Publish the SQ tail and read how many staged SQEs to submit this call.
        let to_submit = self.inner.submission().len() as u32;
        let submitter = self.inner.submitter();
        if let Some(duration) = timeout {
            let timespec = timespec(duration);
            let args = SubmitArgs::new().timespec(&timespec);
            let flags = EnterFlags::EXT_ARG | EnterFlags::GETEVENTS | EnterFlags::NO_IOWAIT;
            // SAFETY: `args` outlives the call; the SQ is synced and holds
            // `to_submit` valid SQEs.
            unsafe { submitter.enter(to_submit, want_sqe as u32, flags.bits(), Some(&args)) }
        } else {
            let flags = EnterFlags::GETEVENTS | EnterFlags::NO_IOWAIT;
            // SAFETY: the SQ is synced and holds `to_submit` valid SQEs; no arg
            // payload is referenced.
            unsafe {
                submitter.enter::<libc::sigset_t>(to_submit, want_sqe as u32, flags.bits(), None)
            }
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
        let cqueue = self.inner.completion();
        let has_entry = !cqueue.is_empty();
        for entry in cqueue {
            match entry.user_data() {
                Self::CANCEL => {}
                Self::NOTIFY => {
                    let flags = entry.flags();
                    if !more(flags) {
                        self.flags.insert(DriverFlags::NEED_PUSH_NOTIFIER);
                    }
                    if let Err(e) = self.notifier.clear() {
                        error!("failed to clear notifier: {e:?}");
                    }
                }
                key => {
                    let flags = entry.flags();
                    if more(flags) {
                        let key = unsafe { BorrowedKey::from_raw(key as _) };
                        let mut key = key.borrow();
                        let mut extra: crate::sys::Extra = IourExtra::new().into();
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

    pub(in crate::sys) fn default_extra(&self) -> IourExtra {
        IourExtra::new()
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
                    match self.submit_auto(Some(Duration::ZERO), true) {
                        Ok(()) => {}
                        Err(e)
                            if matches!(
                                e.kind(),
                                io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
                            ) => {}
                        Err(e) => return Err(e),
                    }
                    // If the CQEs are consumed here, we should make the driver aware of it. We
                    // should not mask `awake` here, otherwise the driver may wait for the next
                    // event indefinitely.
                    //
                    // Anyway it is not a hot path, so we can afford an extra `write` syscall here.
                    self.poll_entries();
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
            let res = catch_unwind_io(AssertUnwindSafe(|| key.as_mut().carrier.call_blocking()));
            let _ = completed.send(Entry::new(key.into_inner(), res));
            waker.wake();
        };
        while let Err(e) = self.pool.dispatch(closure) {
            closure = e.0;
            std::thread::yield_now();
        }
    }

    pub fn flush(&mut self) -> bool {
        let succeed = self.submit_auto(Some(Duration::ZERO), false).is_ok();
        // If submission failed, return true to let the driver wake up immediately.
        !succeed | self.notifier.reset()
    }

    pub fn poll(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        instrument!(compio_log::Level::TRACE, "poll", ?timeout);

        if self.poll_blocking() {
            return Ok(());
        }

        trace!("start polling");

        let need_wait = !self.notifier.reset();

        if self.flags.contains(DriverFlags::NEED_PUSH_NOTIFIER) {
            #[allow(clippy::useless_conversion)]
            self.push_raw(
                PollAdd::new(Fd(self.notifier.as_raw_fd()), libc::POLLIN as _)
                    .multi(true)
                    .build()
                    .user_data(Self::NOTIFY)
                    .into(),
            )?;
            self.flags.remove(DriverFlags::NEED_PUSH_NOTIFIER);
        }

        self.submit_auto(timeout, need_wait)?;

        self.notifier.set_awake();
        self.poll_entries();
        self.notifier.set_awake();

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

        // Close the io_uring ring *before* freeing the remaining in-flight
        // keys. Closing the ring fd makes the kernel wait for in-flight ops to
        // finish or be cancelled, so it will no longer read from or write to
        // the buffers owned by those keys. Without this, the kernel could
        // touch a freed (and potentially recycled) heap allocation, which
        // corrupts the glibc heap and typically surfaces as
        // `malloc_consolidate(): unaligned fastbin chunk detected` /
        // `corrupted double-linked list` during thread shutdown.
        unsafe { ManuallyDrop::drop(&mut self.inner) };

        // Free remaining in-flight keys. Safe now that the kernel is done.
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
        // ENOBUFS indicates the io_uring buffer pool has no available buffer.
        if -result == libc::ENOBUFS {
            Err(io::Error::new(
                io::ErrorKind::ResourceBusy,
                "buffer ring has no available buffer",
            ))
        } else {
            Err(io::Error::from_raw_os_error(-result))
        }
    } else {
        Ok(result as _)
    }
}

fn timespec(duration: std::time::Duration) -> Timespec {
    Timespec::new()
        .sec(duration.as_secs())
        .nsec(duration.subsec_nanos())
}
