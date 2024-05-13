use std::{
    collections::{HashMap, HashSet},
    io,
    mem::ManuallyDrop,
    os::{
        raw::c_void,
        windows::{
            io::{OwnedHandle, OwnedSocket},
            prelude::{AsRawHandle, AsRawSocket},
        },
    },
    pin::Pin,
    ptr::{null, NonNull},
    sync::Arc,
    task::{Poll, Waker},
    time::Duration,
};

use compio_buf::BufResult;
use compio_log::{instrument, trace};
use slab::Slab;
use windows_sys::Win32::{
    Foundation::{ERROR_BUSY, ERROR_OPERATION_ABORTED, ERROR_TIMEOUT, WAIT_OBJECT_0, WAIT_TIMEOUT},
    Networking::WinSock::{WSACleanup, WSAStartup, WSADATA},
    System::{
        Threading::{
            CloseThreadpoolWait, CreateThreadpoolWait, SetThreadpoolWait,
            WaitForThreadpoolWaitCallbacks, PTP_CALLBACK_INSTANCE, PTP_WAIT,
        },
        IO::OVERLAPPED,
    },
};

use crate::{syscall, AsyncifyPool, Entry, OutEntries, ProactorBuilder, PushEntry};

pub(crate) mod op;

mod cp;

pub(crate) use windows_sys::Win32::Networking::WinSock::{
    socklen_t, SOCKADDR_STORAGE as sockaddr_storage,
};

/// On windows, handle and socket are in the same size.
/// Both of them could be attached to an IOCP.
/// Therefore, both could be seen as fd.
pub type RawFd = isize;

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
pub trait OpCode {
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
    /// # Safety
    ///
    /// * `self` must be alive until the operation completes.
    /// * Should not use [`Overlapped::op`].
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>>;

    /// Cancel the async IO operation.
    ///
    /// Usually it calls `CancelIoEx`.
    ///
    /// # Safety
    ///
    /// * Should not use [`Overlapped::op`].
    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        let _optr = optr; // ignore it
        Ok(())
    }
}

/// Low-level driver of IOCP.
pub(crate) struct Driver {
    port: cp::Port,
    waits: HashMap<usize, WinThreadpollWait>,
    cancelled: HashSet<usize>,
    pool: AsyncifyPool,
    notify_overlapped: Arc<Overlapped<()>>,
}

impl Driver {
    const NOTIFY: usize = usize::MAX;

    pub fn new(builder: &ProactorBuilder) -> io::Result<Self> {
        instrument!(compio_log::Level::TRACE, "new", ?builder);
        let mut data: WSADATA = unsafe { std::mem::zeroed() };
        syscall!(SOCKET, WSAStartup(0x202, &mut data))?;

        let port = cp::Port::new()?;
        let driver = port.as_raw_handle() as _;
        Ok(Self {
            port,
            waits: HashMap::default(),
            cancelled: HashSet::default(),
            pool: builder.create_or_get_thread_pool(),
            notify_overlapped: Arc::new(Overlapped::new(driver, Self::NOTIFY, ())),
        })
    }

    pub fn create_op<T: OpCode + 'static>(&self, user_data: usize, op: T) -> RawOp {
        RawOp::new(self.port.as_raw_handle() as _, user_data, op)
    }

    pub fn attach(&mut self, fd: RawFd) -> io::Result<()> {
        self.port.attach(fd)
    }

    pub fn cancel(&mut self, user_data: usize, registry: &mut Slab<RawOp>) {
        instrument!(compio_log::Level::TRACE, "cancel", user_data);
        trace!("cancel RawOp");
        self.cancelled.insert(user_data);
        if let Some(op) = registry.get_mut(user_data) {
            let overlapped_ptr = op.as_mut_ptr();
            let op = op.as_op_pin();
            // It's OK to fail to cancel.
            trace!("call OpCode::cancel");
            unsafe { op.cancel(overlapped_ptr.cast()) }.ok();
        }
    }

    pub fn push(&mut self, user_data: usize, op: &mut RawOp) -> Poll<io::Result<usize>> {
        instrument!(compio_log::Level::TRACE, "push", user_data);
        if self.cancelled.remove(&user_data) {
            trace!("pushed RawOp already cancelled");
            Poll::Ready(Err(io::Error::from_raw_os_error(
                ERROR_OPERATION_ABORTED as _,
            )))
        } else {
            trace!("push RawOp");
            let optr = op.as_mut_ptr();
            let op_pin = op.as_op_pin();
            match op_pin.op_type() {
                OpType::Overlapped => unsafe { op_pin.operate(optr.cast()) },
                OpType::Blocking => {
                    if self.push_blocking(op)? {
                        Poll::Pending
                    } else {
                        Poll::Ready(Err(io::Error::from_raw_os_error(ERROR_BUSY as _)))
                    }
                }
                OpType::Event(e) => {
                    self.waits.insert(
                        user_data,
                        WinThreadpollWait::new(self.port.handle(), e, op)?,
                    );
                    Poll::Pending
                }
            }
        }
    }

    fn push_blocking(&mut self, op: &mut RawOp) -> io::Result<bool> {
        // Safety: the RawOp is not released before the operation returns.
        struct SendWrapper<T>(T);
        unsafe impl<T> Send for SendWrapper<T> {}

        let optr = SendWrapper(NonNull::from(op));
        let port = self.port.handle();
        Ok(self
            .pool
            .dispatch(move || {
                #[allow(clippy::redundant_locals)]
                let mut optr = optr;
                // Safety: the pointer is created from a reference.
                let op = unsafe { optr.0.as_mut() };
                let optr = op.as_mut_ptr();
                let res = op.operate_blocking();
                port.post(res, optr).ok();
            })
            .is_ok())
    }

    fn create_entry(
        cancelled: &mut HashSet<usize>,
        waits: &mut HashMap<usize, WinThreadpollWait>,
        entry: Entry,
    ) -> Option<Entry> {
        let user_data = entry.user_data();
        if user_data != Self::NOTIFY {
            waits.remove(&user_data);
            let result = if cancelled.remove(&user_data) {
                Err(io::Error::from_raw_os_error(ERROR_OPERATION_ABORTED as _))
            } else {
                entry.into_result()
            };
            Some(Entry::new(user_data, result))
        } else {
            None
        }
    }

    pub unsafe fn poll(
        &mut self,
        timeout: Option<Duration>,
        mut entries: OutEntries<impl Extend<usize>>,
    ) -> io::Result<()> {
        instrument!(compio_log::Level::TRACE, "poll", ?timeout);

        entries.extend(
            self.port
                .poll(timeout)?
                .filter_map(|e| Self::create_entry(&mut self.cancelled, &mut self.waits, e)),
        );

        Ok(())
    }

    pub fn handle(&self) -> io::Result<NotifyHandle> {
        Ok(NotifyHandle::new(
            self.port.handle(),
            self.notify_overlapped.clone(),
        ))
    }
}

impl AsRawFd for Driver {
    fn as_raw_fd(&self) -> RawFd {
        self.port.as_raw_handle() as _
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        syscall!(SOCKET, WSACleanup()).ok();
    }
}

/// A notify handle to the inner driver.
pub struct NotifyHandle {
    port: cp::PortHandle,
    overlapped: Arc<Overlapped<()>>,
}

impl NotifyHandle {
    fn new(port: cp::PortHandle, overlapped: Arc<Overlapped<()>>) -> Self {
        Self { port, overlapped }
    }

    /// Notify the inner driver.
    pub fn notify(&self) -> io::Result<()> {
        self.port.post_raw(self.overlapped.as_ref())
    }
}

struct WinThreadpollWait {
    wait: PTP_WAIT,
    // For memory safety.
    #[allow(dead_code)]
    context: Box<WinThreadpollWaitContext>,
}

impl WinThreadpollWait {
    pub fn new(port: cp::PortHandle, event: RawFd, op: &mut RawOp) -> io::Result<Self> {
        let mut context = Box::new(WinThreadpollWaitContext { port, op });
        let wait = syscall!(
            BOOL,
            CreateThreadpoolWait(
                Some(Self::wait_callback),
                (&mut *context) as *mut WinThreadpollWaitContext as _,
                null()
            )
        )?;
        unsafe {
            SetThreadpoolWait(wait, event as _, null());
        }
        Ok(Self { wait, context })
    }

    unsafe extern "system" fn wait_callback(
        _instance: PTP_CALLBACK_INSTANCE,
        context: *mut c_void,
        _wait: PTP_WAIT,
        result: u32,
    ) {
        let context = &*(context as *mut WinThreadpollWaitContext);
        let res = match result {
            WAIT_OBJECT_0 => Ok(0),
            WAIT_TIMEOUT => Err(io::Error::from_raw_os_error(ERROR_TIMEOUT as _)),
            _ => Err(io::Error::from_raw_os_error(result as _)),
        };
        let res = if res.is_err() {
            res
        } else {
            let op = unsafe { &mut *context.op };
            op.operate_blocking()
        };
        context.port.post(res, (*context.op).as_mut_ptr()).ok();
    }
}

impl Drop for WinThreadpollWait {
    fn drop(&mut self) {
        unsafe {
            SetThreadpoolWait(self.wait, 0, null());
            WaitForThreadpoolWaitCallbacks(self.wait, 1);
            CloseThreadpoolWait(self.wait);
        }
    }
}

struct WinThreadpollWaitContext {
    port: cp::PortHandle,
    op: *mut RawOp,
}

/// The overlapped struct we actually used for IOCP.
#[repr(C)]
pub struct Overlapped<T: ?Sized> {
    /// The base [`OVERLAPPED`].
    pub base: OVERLAPPED,
    /// The unique ID of created driver.
    pub driver: RawFd,
    /// The registered user defined data.
    pub user_data: usize,
    /// The opcode.
    /// The user should guarantee the type is correct.
    pub op: T,
}

impl<T> Overlapped<T> {
    pub(crate) fn new(driver: RawFd, user_data: usize, op: T) -> Self {
        Self {
            base: unsafe { std::mem::zeroed() },
            driver,
            user_data,
            op,
        }
    }
}

// SAFETY: neither field of `OVERLAPPED` is used
unsafe impl Send for Overlapped<()> {}
unsafe impl Sync for Overlapped<()> {}

pub(crate) struct RawOp {
    op: NonNull<Overlapped<dyn OpCode>>,
    // The two flags here are manual reference counting. The driver holds the strong ref until it
    // completes; the runtime holds the strong ref until the future is dropped.
    cancelled: bool,
    result: PushEntry<Option<Waker>, io::Result<usize>>,
}

impl RawOp {
    pub(crate) fn new(driver: RawFd, user_data: usize, op: impl OpCode + 'static) -> Self {
        let op = Overlapped::new(driver, user_data, op);
        let op = Box::new(op) as Box<Overlapped<dyn OpCode>>;
        Self {
            op: unsafe { NonNull::new_unchecked(Box::into_raw(op)) },
            cancelled: false,
            result: PushEntry::Pending(None),
        }
    }

    pub fn as_op_pin(&mut self) -> Pin<&mut dyn OpCode> {
        unsafe { Pin::new_unchecked(&mut self.op.as_mut().op) }
    }

    pub fn as_mut_ptr(&mut self) -> *mut Overlapped<dyn OpCode> {
        self.op.as_ptr()
    }

    pub fn set_cancelled(&mut self) -> bool {
        self.cancelled = true;
        self.has_result()
    }

    pub fn set_result(&mut self, res: io::Result<usize>) -> bool {
        if let PushEntry::Pending(Some(w)) =
            std::mem::replace(&mut self.result, PushEntry::Ready(res))
        {
            w.wake();
        }
        self.cancelled
    }

    pub fn has_result(&self) -> bool {
        self.result.is_ready()
    }

    pub fn set_waker(&mut self, waker: Waker) {
        if let PushEntry::Pending(w) = &mut self.result {
            *w = Some(waker)
        }
    }

    /// # Safety
    /// The caller should ensure the correct type.
    ///
    /// # Panics
    /// This function will panic if the result has not been set.
    pub unsafe fn into_inner<T: OpCode>(self) -> BufResult<usize, T> {
        let mut this = ManuallyDrop::new(self);
        let overlapped: Box<Overlapped<T>> = Box::from_raw(this.op.cast().as_ptr());
        BufResult(
            std::mem::replace(&mut this.result, PushEntry::Pending(None))
                .take_ready()
                .unwrap(),
            overlapped.op,
        )
    }

    fn operate_blocking(&mut self) -> io::Result<usize> {
        let optr = self.as_mut_ptr();
        let op = self.as_op_pin();
        let res = unsafe { op.operate(optr.cast()) };
        match res {
            Poll::Pending => unreachable!("this operation is not overlapped"),
            Poll::Ready(res) => res,
        }
    }
}

impl Drop for RawOp {
    fn drop(&mut self) {
        if self.has_result() {
            let _ = unsafe { Box::from_raw(self.op.as_ptr()) };
        }
    }
}
