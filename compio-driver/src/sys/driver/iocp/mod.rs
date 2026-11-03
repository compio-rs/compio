use std::{
    collections::{HashMap, HashSet},
    marker::PhantomData,
    os::windows::io::AsRawHandle,
    sync::Arc,
    time::{Duration, Instant},
};

use flume::{Receiver, Sender};
use windows_sys::Win32::{Foundation::ERROR_OPERATION_ABORTED, System::IO::OVERLAPPED};

use crate::{
    AsyncifyPool, DriverType, Entry, ErasedKey, ProactorBuilder,
    control::Carrier,
    key::BorrowedKey,
    sys::{driver::AwakeFlag, extra::IocpExtra, prelude::*},
};

mod cp;
mod wait;

mod_use![op];

/// Operation type.
pub enum OpType {
    /// An overlapped operation.
    Overlapped,
    /// A blocking operation, needs a thread to spawn. The `operate` method
    /// should be thread safe.
    Blocking,
    /// A Win32 event object to be waited. The user should ensure that the
    /// handle is valid till operation completes. The `operate` method should be
    /// thread safe.
    Event(RawFd),
}

/// Low-level driver of IOCP.
pub(crate) struct Driver {
    notify: Arc<Notify>,
    waits: HashMap<usize, wait::Wait>,
    /// The operations the I/O manager is currently executing.
    in_flight: HashSet<*mut Overlapped>,
    pool: AsyncifyPool,
    completed_tx: Sender<Entry>,
    completed_rx: Receiver<Entry>,
    _local_marker: PhantomData<ErasedKey>,
}

impl Driver {
    pub fn new(builder: &ProactorBuilder) -> io::Result<Self> {
        instrument!(compio_log::Level::TRACE, "new", ?builder);

        let port = cp::Port::new()?;
        let driver = port.as_raw_handle() as _;
        let overlapped = Overlapped::new(driver);
        let notify = Arc::new(Notify::new(port, overlapped));
        let (completed_tx, completed_rx) = flume::unbounded();

        Ok(Self {
            notify,
            completed_tx,
            completed_rx,
            waits: HashMap::default(),
            in_flight: HashSet::default(),
            pool: builder.create_or_get_thread_pool(),
            _local_marker: PhantomData,
        })
    }

    pub fn driver_type(&self) -> DriverType {
        DriverType::IOCP
    }

    fn port(&self) -> &cp::Port {
        &self.notify.port
    }

    pub(in crate::sys) fn default_extra(&self) -> IocpExtra {
        IocpExtra::new(self.port().as_raw_handle() as _)
    }

    pub fn attach(&mut self, fd: RawFd) -> io::Result<()> {
        self.port().attach(fd)
    }

    pub fn cancel(&mut self, key: ErasedKey) {
        instrument!(compio_log::Level::TRACE, "cancel", ?key);
        trace!("cancel RawOp");
        let optr = key.borrow().extra_mut().optr();
        if let Some(w) = self.waits.get_mut(&key.as_raw())
            && w.cancel().is_ok()
        {
            // The pack has been cancelled successfully, which means no packet
            // will be post to IOCP. Need not set the result because
            // `create_entry` handles it.
            self.port().post_raw(optr).ok();
        }
        trace!("call OpCode::cancel");
        // It's OK to fail to cancel.
        key.borrow().carrier.cancel(optr.cast()).ok();
    }

    pub fn push(&mut self, key: ErasedKey) -> Poll<io::Result<usize>> {
        instrument!(compio_log::Level::TRACE, "push", ?key);
        trace!("push RawOp");
        let mut op = key.borrow();
        let optr = op.extra_mut().optr();
        let op_type = op.carrier.op_type();
        match op_type {
            OpType::Overlapped => unsafe {
                let res = op.carrier.operate(optr.cast());
                let owned = std::ptr::eq(op.extra().driver(), self.port().as_raw_handle());
                drop(op);
                if res.is_pending() {
                    let _ = key.into_raw();
                    if owned {
                        self.in_flight.insert(optr);
                    }
                }
                res
            },
            OpType::Blocking => {
                drop(op);
                self.push_blocking(key);
                Poll::Pending
            }
            OpType::Event(e) => {
                drop(op);
                self.waits
                    .insert(key.as_raw(), wait::Wait::new(self.notify.clone(), e, key)?);
                Poll::Pending
            }
        }
    }

    fn push_blocking(&mut self, key: ErasedKey) {
        let notify = self.notify.clone();
        let tx = self.completed_tx.clone();

        // SAFETY: we're submitting into the driver, so it's safe to freeze
        // here.
        let mut key = unsafe { key.freeze() };

        let mut closure = move || {
            let res = key.as_mut().operate_blocking();
            let entry = Entry::new(key.into_inner(), res);
            _ = tx.send(entry);
            notify.wake();
        };

        while let Err(e) = self.pool.dispatch(closure) {
            closure = e.0;
            std::thread::yield_now();
        }
    }

    pub fn flush(&mut self) -> bool {
        self.notify.reset()
    }

    fn create_entry(
        notify: *const Overlapped,
        in_flight: &mut HashSet<*mut Overlapped>,
        waits: &mut HashMap<usize, wait::Wait>,
        entry: cp::RawEntry,
    ) -> Option<Entry> {
        let optr = entry.overlapped;
        if optr.cast_const() == notify {
            return None;
        }

        // The I/O manager has released the operation. Event waits are not
        // tracked in `in_flight`.
        in_flight.remove(&optr);

        let entry = Entry::new(unsafe { ErasedKey::from_optr(optr) }, entry.result);

        // if there's no wait, just return the entry
        let Some(w) = waits.remove(&entry.user_data()) else {
            return Some(entry);
        };

        let entry = if w.is_cancelled() {
            Entry::new(
                entry.into_key(),
                Err(io::Error::from_raw_os_error(ERROR_OPERATION_ABORTED as _)),
            )
        } else if entry.result.is_err() {
            entry
        } else {
            let key = entry.into_key();
            let result = key.borrow().operate_blocking();
            Entry::new(key, result)
        };

        Some(entry)
    }

    pub fn poll(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        instrument!(compio_log::Level::TRACE, "poll", ?timeout);

        let notify = &self.notify.overlapped as *const Overlapped;

        let mut has_entry = false;
        while let Ok(entry) = self.completed_rx.try_recv() {
            entry.notify();
            has_entry = true;
        }
        if self.notify.reset() {
            has_entry = true;
        }

        let timeout = if has_entry {
            Some(Duration::ZERO)
        } else {
            timeout
        };
        let result = self.notify.port.poll(timeout).map(|entries| {
            for e in entries {
                if let Some(e) = Self::create_entry(notify, &mut self.in_flight, &mut self.waits, e)
                {
                    self.notify.set_awake();
                    e.notify()
                }
            }
        });
        self.notify.set_awake();

        match result {
            Ok(_) => Ok(()),
            Err(e) if has_entry && e.kind() == io::ErrorKind::TimedOut => Ok(()),
            Err(e) => Err(e),
        }
    }

    pub fn waker(&self) -> Waker {
        Waker::from(self.notify.clone())
    }

    pub fn pop_multishot(&mut self, _: &ErasedKey) -> Option<BufResult<usize, crate::sys::Extra>> {
        None
    }
}

impl AsRawFd for Driver {
    fn as_raw_fd(&self) -> RawFd {
        self.port().as_raw_handle() as _
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        const DROP_TIMEOUT: Duration = Duration::from_secs(1);

        for &optr in &self.in_flight {
            // SAFETY: `push` leaked the key and it is still in flight.
            let key = unsafe { BorrowedKey::from_optr(optr) };
            let mut op = key.borrow();
            if let Err(e) = op.carrier.cancel(optr.cast()) {
                trace!("failed to cancel in-flight op on drop: {e:?}");
            }
        }

        let deadline = Instant::now() + DROP_TIMEOUT;
        let notify = &self.notify.overlapped as *const Overlapped;

        while !self.in_flight.is_empty() {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            let timeout = (deadline - now).max(Duration::from_millis(1));

            let entries = match self.notify.port.poll(Some(timeout)) {
                Ok(entries) => entries,
                Err(e) => {
                    warn!("failed to poll IOCP while dropping the driver: {e:?}");
                    break;
                }
            };

            for entry in entries {
                let optr = entry.overlapped;
                if optr.cast_const() == notify {
                    continue;
                }

                if self.in_flight.remove(&optr) {
                    drop(unsafe { ErasedKey::from_optr(optr) });
                } else {
                    trace!("ignore untracked completion while dropping the driver");
                }
            }
        }

        if !self.in_flight.is_empty() {
            warn!(
                "leaking {} in-flight IOCP op(s) on driver drop; the I/O manager did not finish \
                 cancelling them within {DROP_TIMEOUT:?}",
                self.in_flight.len()
            );
        }

        let mut leaked = 0;
        for (key, w) in std::mem::take(&mut self.waits) {
            if w.cancel_for_drop() {
                drop(unsafe { ErasedKey::from_raw(key) });
            } else {
                leaked += 1;
            }
        }
        if leaked > 0 {
            warn!("leaking {leaked} pending IOCP event wait(s) on driver drop");
        }
    }
}

/// A notify handle to the inner driver.
pub(crate) struct Notify {
    port: cp::Port,
    overlapped: Overlapped,
    awake: AwakeFlag,
}

impl Notify {
    fn new(port: cp::Port, overlapped: Overlapped) -> Self {
        Self {
            port,
            overlapped,
            awake: AwakeFlag::new(),
        }
    }

    fn set_awake(&self) {
        self.awake.set();
    }

    fn reset(&self) -> bool {
        self.awake.reset()
    }
}

impl Wake for Notify {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        if !self.awake.wake() {
            self.port.post_raw(&self.overlapped).ok();
        }
    }
}
