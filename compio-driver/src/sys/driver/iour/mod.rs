use std::{
    collections::HashSet, marker::PhantomData, panic::AssertUnwindSafe, sync::Arc, time::Duration,
};

use crate::sys::{extra::IourExtra, prelude::*};

mod_use![op, notify];

cfg_if! {
    if #[cfg(feature = "io-uring-cqe32")] {
        use io_uring::cqueue::Entry32 as CEntry;
    } else {
        use io_uring::cqueue::Entry as CEntry;
    }
}

cfg_if! {
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
    key::{BorrowedKey, ErasedKey},
    panic::catch_unwind_io,
};

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
            if let Some(cpu) = builder.sqpoll_cpu {
                io_uring_builder.setup_sqpoll_cpu(cpu);
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

    pub(in crate::sys) fn inner(&mut self) -> &mut IoUring<SEntry, CEntry> {
        &mut self.inner
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
        let mut cqueue = self.inner.completion();
        cqueue.sync();
        let has_entry = !cqueue.is_empty();
        self.notifier.set_awake(true);
        for entry in cqueue {
            match entry.user_data() {
                Self::CANCEL => {}
                Self::NOTIFY => {
                    let flags = entry.flags();
                    if !more(flags) {
                        self.need_push_notifier = true;
                    }
                    if let Err(_e) = self.notifier.clear() {
                        error!("failed to clear notifier: {_e}");
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
        self.notifier.set_awake(false);
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
                    match self.submit_auto(Some(Duration::ZERO)) {
                        Ok(()) => {}
                        Err(e)
                            if matches!(
                                e.kind(),
                                io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
                            ) => {}
                        Err(e) => return Err(e),
                    }
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

    pub fn poll(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        instrument!(compio_log::Level::TRACE, "poll", ?timeout);

        if self.poll_blocking() {
            return Ok(());
        }

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

        self.submit_auto(timeout)?;
        self.poll_entries();

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
