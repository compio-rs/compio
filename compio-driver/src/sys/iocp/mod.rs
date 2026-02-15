use std::{
    collections::HashMap,
    io,
    os::windows::io::{
        AsHandle, AsRawHandle, AsRawSocket, AsSocket, BorrowedHandle, BorrowedSocket, OwnedHandle,
        OwnedSocket,
    },
    pin::Pin,
    sync::Arc,
    task::{Poll, Wake, Waker},
    time::Duration,
};

use compio_log::{instrument, trace};
use windows_sys::Win32::{
    Foundation::{ERROR_CANCELLED, HANDLE},
    System::IO::OVERLAPPED,
};

use crate::{AsyncifyPool, BufferPool, DriverType, Entry, ErasedKey, ProactorBuilder, key::RefExt};

pub(crate) mod op;

mod cp;
mod wait;

/// Extra data attached for IOCP.
#[repr(C)]
pub struct Extra {
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

/// On windows, handle and socket are in the same size.
/// Both of them could be attached to an IOCP.
/// Therefore, both could be seen as fd.
pub type RawFd = HANDLE;

/// Extracts raw fds.
pub trait AsRawFd {
    /// Extracts the raw fd.
    fn as_raw_fd(&self) -> RawFd;
}

/// Owned handle or socket on Windows.
#[derive(Debug)]
pub enum OwnedFd {
    /// Win32 handle.
    File(OwnedHandle),
    /// Windows socket handle.
    Socket(OwnedSocket),
}

impl AsRawFd for OwnedFd {
    fn as_raw_fd(&self) -> RawFd {
        match self {
            Self::File(fd) => fd.as_raw_handle() as _,
            Self::Socket(s) => s.as_raw_socket() as _,
        }
    }
}

impl AsRawFd for RawFd {
    fn as_raw_fd(&self) -> RawFd {
        *self
    }
}

impl AsRawFd for std::fs::File {
    fn as_raw_fd(&self) -> RawFd {
        self.as_raw_handle() as _
    }
}

impl AsRawFd for OwnedHandle {
    fn as_raw_fd(&self) -> RawFd {
        self.as_raw_handle() as _
    }
}

impl AsRawFd for socket2::Socket {
    fn as_raw_fd(&self) -> RawFd {
        self.as_raw_socket() as _
    }
}

impl AsRawFd for OwnedSocket {
    fn as_raw_fd(&self) -> RawFd {
        self.as_raw_socket() as _
    }
}

impl AsRawFd for std::process::ChildStdin {
    fn as_raw_fd(&self) -> RawFd {
        self.as_raw_handle() as _
    }
}

impl AsRawFd for std::process::ChildStdout {
    fn as_raw_fd(&self) -> RawFd {
        self.as_raw_handle() as _
    }
}

impl AsRawFd for std::process::ChildStderr {
    fn as_raw_fd(&self) -> RawFd {
        self.as_raw_handle() as _
    }
}

impl From<OwnedHandle> for OwnedFd {
    fn from(value: OwnedHandle) -> Self {
        Self::File(value)
    }
}

impl From<std::fs::File> for OwnedFd {
    fn from(value: std::fs::File) -> Self {
        Self::File(OwnedHandle::from(value))
    }
}

impl From<std::process::ChildStdin> for OwnedFd {
    fn from(value: std::process::ChildStdin) -> Self {
        Self::File(OwnedHandle::from(value))
    }
}

impl From<std::process::ChildStdout> for OwnedFd {
    fn from(value: std::process::ChildStdout) -> Self {
        Self::File(OwnedHandle::from(value))
    }
}

impl From<std::process::ChildStderr> for OwnedFd {
    fn from(value: std::process::ChildStderr) -> Self {
        Self::File(OwnedHandle::from(value))
    }
}

impl From<OwnedSocket> for OwnedFd {
    fn from(value: OwnedSocket) -> Self {
        Self::Socket(value)
    }
}

impl From<socket2::Socket> for OwnedFd {
    fn from(value: socket2::Socket) -> Self {
        Self::Socket(OwnedSocket::from(value))
    }
}

/// Borrowed handle or socket on Windows.
#[derive(Debug)]
pub enum BorrowedFd<'a> {
    /// Win32 handle.
    File(BorrowedHandle<'a>),
    /// Windows socket handle.
    Socket(BorrowedSocket<'a>),
}

impl AsRawFd for BorrowedFd<'_> {
    fn as_raw_fd(&self) -> RawFd {
        match self {
            Self::File(fd) => fd.as_raw_handle() as RawFd,
            Self::Socket(s) => s.as_raw_socket() as RawFd,
        }
    }
}

impl<'a> From<BorrowedHandle<'a>> for BorrowedFd<'a> {
    fn from(value: BorrowedHandle<'a>) -> Self {
        Self::File(value)
    }
}

impl<'a> From<BorrowedSocket<'a>> for BorrowedFd<'a> {
    fn from(value: BorrowedSocket<'a>) -> Self {
        Self::Socket(value)
    }
}

/// Extracts fds.
pub trait AsFd {
    /// Extracts the borrowed fd.
    fn as_fd(&self) -> BorrowedFd<'_>;
}

impl AsFd for OwnedFd {
    fn as_fd(&self) -> BorrowedFd<'_> {
        match self {
            Self::File(fd) => fd.as_fd(),
            Self::Socket(s) => s.as_fd(),
        }
    }
}

impl AsFd for std::fs::File {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.as_handle().into()
    }
}

impl AsFd for OwnedHandle {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.as_handle().into()
    }
}

impl AsFd for BorrowedHandle<'_> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        (*self).into()
    }
}

impl AsFd for socket2::Socket {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.as_socket().into()
    }
}

impl AsFd for OwnedSocket {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.as_socket().into()
    }
}

impl AsFd for BorrowedSocket<'_> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        (*self).into()
    }
}

impl AsFd for std::process::ChildStdin {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.as_handle().into()
    }
}

impl AsFd for std::process::ChildStdout {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.as_handle().into()
    }
}

impl AsFd for std::process::ChildStderr {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.as_handle().into()
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
    /// Determines that the operation is really overlapped defined by Windows
    /// API. If not, the driver will try to operate it in another thread.
    fn op_type(&self) -> OpType {
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
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>>;

    /// Cancel the async IO operation.
    ///
    /// Usually it calls `CancelIoEx`.
    // # Safety for implementors
    //
    // `optr` must not be dereferenced. It's only used as a marker to identify the
    // operation.
    fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        let _optr = optr; // ignore it
        Ok(())
    }
}

/// Low-level driver of IOCP.
pub(crate) struct Driver {
    notify: Arc<Notify>,
    waits: HashMap<usize, wait::Wait>,
    pool: AsyncifyPool,
}

impl Driver {
    pub fn new(builder: &ProactorBuilder) -> io::Result<Self> {
        instrument!(compio_log::Level::TRACE, "new", ?builder);

        let port = cp::Port::new()?;
        let driver = port.as_raw_handle() as _;
        let overlapped = Overlapped::new(driver);
        let notify = Arc::new(Notify::new(port, overlapped));
        Ok(Self {
            notify,
            waits: HashMap::default(),
            pool: builder.create_or_get_thread_pool(),
        })
    }

    pub fn driver_type(&self) -> DriverType {
        DriverType::IOCP
    }

    fn port(&self) -> &cp::Port {
        &self.notify.port
    }

    pub fn default_extra(&self) -> Extra {
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
        key.borrow().pinned_op().cancel(optr.cast()).ok();
    }

    pub fn push(&mut self, key: ErasedKey) -> Poll<io::Result<usize>> {
        instrument!(compio_log::Level::TRACE, "push", ?key);
        trace!("push RawOp");
        let mut op = key.borrow();
        let optr = op.extra_mut().optr();
        let pinned = op.pinned_op();
        let op_type = pinned.op_type();
        match op_type {
            OpType::Overlapped => unsafe {
                let res = pinned.operate(optr.cast());
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
        // SAFETY: we're submitting into the driver, so it's safe to freeze here.
        let mut key = unsafe { key.freeze() };

        let mut closure = move || {
            let op = key.as_mut();
            let res = op.operate_blocking();
            let optr = op.extra_mut().optr();
            // key will be unfronzen in `create_entry` when the result is ready
            notify.port.post(res, optr).ok();
        };

        while let Err(e) = self.pool.dispatch(closure) {
            closure = e.0;
        }
    }

    fn create_entry(
        notify: *const Overlapped,
        waits: &mut HashMap<usize, wait::Wait>,
        entry: cp::RawEntry,
    ) -> Option<Entry> {
        // Ignore existing entries
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

        for e in self.notify.port.poll(timeout)? {
            if let Some(e) = Self::create_entry(notify, &mut self.waits, e) {
                e.notify()
            }
        }

        Ok(())
    }

    pub fn waker(&self) -> Waker {
        Waker::from(self.notify.clone())
    }

    pub fn create_buffer_pool(
        &mut self,
        buffer_len: u16,
        buffer_size: usize,
    ) -> io::Result<BufferPool> {
        Ok(BufferPool::new(buffer_len, buffer_size))
    }

    /// # Safety
    ///
    /// caller must make sure release the buffer pool with correct driver
    pub unsafe fn release_buffer_pool(&mut self, _: BufferPool) -> io::Result<()> {
        Ok(())
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
