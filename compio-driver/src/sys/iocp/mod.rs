use std::{
    collections::HashMap,
    fmt, io,
    marker::PhantomData,
    os::windows::io::{
        AsHandle, AsRawHandle, AsRawSocket, AsSocket, BorrowedHandle, BorrowedSocket, OwnedHandle,
        OwnedSocket,
    },
    sync::Arc,
    task::{Poll, Wake, Waker},
    time::Duration,
};

use compio_buf::BufResult;
use compio_log::{instrument, trace};
use flume::{Receiver, Sender};
use windows_sys::Win32::{
    Foundation::{ERROR_CANCELLED, HANDLE},
    System::IO::OVERLAPPED,
};

use crate::{AsyncifyPool, DriverType, Entry, ErasedKey, ProactorBuilder, control::Carrier};

pub(crate) mod op;

mod cp;
mod fd;
mod wait;

pub use fd::*;

/// Extra data attached for IOCP.
#[repr(C)]
#[derive(Debug)]
pub(in crate::sys) struct Extra {
    overlapped: Overlapped,
}

impl Default for Extra {
    fn default() -> Self {
        Self {
            overlapped: Overlapped::new(std::ptr::null_mut()),
        }
    }
}

impl Extra {
    pub(crate) fn new(driver: RawFd) -> Self {
        Self {
            overlapped: Overlapped::new(driver),
        }
    }
}

impl super::Extra {
    pub(crate) fn optr(&mut self) -> *mut Overlapped {
        &mut self.0.overlapped as _
    }
}

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

/// Abstraction of IOCP operations.
///
/// # Safety
///
/// Implementors must ensure that the operation is safe to be polled
/// according to the returned [`OpType`].
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

    /// Determines that the operation is really overlapped defined by Windows
    /// API. If not, the driver will try to operate it in another thread.
    fn op_type(&self, control: &Self::Control) -> OpType {
        _ = control;
        OpType::Overlapped
    }

    /// Perform Windows API call with given pointer to overlapped struct.
    ///
    /// It is always safe to cast `optr` to a pointer to
    /// [`Overlapped<Self>`].
    ///
    /// Don't do heavy work here if [`OpCode::op_type`] returns
    /// [`OpType::Event`].
    ///
    /// # Safety
    ///
    /// * `self` must be alive until the operation completes.
    /// * When [`OpCode::op_type`] returns [`OpType::Blocking`], this method is
    ///   called in another thread.
    unsafe fn operate(
        &mut self,
        control: &mut Self::Control,
        optr: *mut OVERLAPPED,
    ) -> Poll<io::Result<usize>>;

    /// Cancel the async IO operation.
    ///
    /// Usually it calls `CancelIoEx`.
    // # Safety for implementors
    //
    // `optr` must not be dereferenced. It's only used as a marker to identify the
    // operation.
    fn cancel(&mut self, control: &mut Self::Control, optr: *mut OVERLAPPED) -> io::Result<()> {
        _ = control;
        _ = optr;
        Ok(())
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
}

pub(crate) trait Carry {
    fn op_type(&self) -> OpType;

    unsafe fn operate(&mut self, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>>;

    fn cancel(&mut self, optr: *mut OVERLAPPED) -> io::Result<()>;

    unsafe fn set_result(&mut self, _: &io::Result<usize>, _: &crate::Extra);
}

impl<T: OpCode> Carry for Carrier<T> {
    fn op_type(&self) -> OpType {
        let (op, control) = self.as_iocp();
        op.op_type(control)
    }

    unsafe fn operate(&mut self, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let (op, control) = self.as_iocp_mut();
        unsafe { op.operate(control, optr) }
    }

    fn cancel(&mut self, optr: *mut OVERLAPPED) -> io::Result<()> {
        let (op, control) = self.as_iocp_mut();
        op.cancel(control, optr)
    }

    unsafe fn set_result(&mut self, res: &io::Result<usize>, extra: &crate::Extra) {
        let (op, control) = self.as_iocp_mut();
        unsafe { op.set_result(control, res, extra) }
    }
}

/// Low-level driver of IOCP.
pub(crate) struct Driver {
    notify: Arc<Notify>,
    waits: HashMap<usize, wait::Wait>,
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

    pub(in crate::sys) fn default_extra(&self) -> Extra {
        Extra::new(self.port().as_raw_handle() as _)
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
            // The pack has been cancelled successfully, which means no packet will be post
            // to IOCP. Need not set the result because `create_entry` handles it.
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
                drop(op);
                if res.is_pending() {
                    key.into_raw();
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

        // SAFETY: we're submitting into the driver, so it's safe to freeze here.
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

    fn create_entry(
        notify: *const Overlapped,
        waits: &mut HashMap<usize, wait::Wait>,
        entry: cp::RawEntry,
    ) -> Option<Entry> {
        if entry.overlapped.cast_const() == notify {
            return None;
        }

        let entry = Entry::new(
            unsafe { ErasedKey::from_optr(entry.overlapped) },
            entry.result,
        );

        // if there's no wait, just return the entry
        let Some(w) = waits.remove(&entry.user_data()) else {
            return Some(entry);
        };

        let entry = if w.is_cancelled() {
            Entry::new(
                entry.into_key(),
                Err(io::Error::from_raw_os_error(ERROR_CANCELLED as _)),
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

        if !has_entry {
            for e in self.notify.port.poll(timeout)? {
                if let Some(e) = Self::create_entry(notify, &mut self.waits, e) {
                    e.notify()
                }
            }
        }

        Ok(())
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

/// A notify handle to the inner driver.
pub(crate) struct Notify {
    port: cp::Port,
    overlapped: Overlapped,
}

impl Notify {
    fn new(port: cp::Port, overlapped: Overlapped) -> Self {
        Self { port, overlapped }
    }

    /// Notify the inner driver.
    pub fn notify(&self) -> io::Result<()> {
        self.port.post_raw(&self.overlapped)
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

/// The overlapped struct we actually used for IOCP.
#[repr(C)]
pub struct Overlapped {
    /// The base [`OVERLAPPED`].
    pub base: OVERLAPPED,
    /// The unique ID of created driver.
    pub driver: RawFd,
}

impl fmt::Debug for Overlapped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Overlapped")
            .field("base", &"OVERLAPPED")
            .field("driver", &self.driver)
            .finish()
    }
}

impl Overlapped {
    pub(crate) fn new(driver: RawFd) -> Self {
        Self {
            base: unsafe { std::mem::zeroed() },
            driver,
        }
    }
}

// SAFETY: neither field of `OVERLAPPED` is used
unsafe impl Send for Overlapped {}
unsafe impl Sync for Overlapped {}
