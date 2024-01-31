#[cfg(feature = "once_cell_try")]
use std::sync::OnceLock;
use std::{
    collections::{HashSet, VecDeque},
    io,
    mem::ManuallyDrop,
    os::windows::prelude::{
        AsRawHandle, AsRawSocket, FromRawHandle, FromRawSocket, IntoRawHandle, IntoRawSocket,
        OwnedHandle, RawHandle,
    },
    pin::Pin,
    ptr::NonNull,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Condvar, Mutex, MutexGuard,
    },
    task::Poll,
    time::Duration,
};

use compio_buf::{arrayvec::ArrayVec, BufResult};
use compio_log::{instrument, trace};
use crossbeam_skiplist::SkipMap;
#[cfg(not(feature = "once_cell_try"))]
use once_cell::sync::OnceCell as OnceLock;
use slab::Slab;
use windows_sys::Win32::{
    Foundation::{
        RtlNtStatusToDosError, ERROR_BAD_COMMAND, ERROR_BUSY, ERROR_HANDLE_EOF,
        ERROR_IO_INCOMPLETE, ERROR_NO_DATA, ERROR_OPERATION_ABORTED, ERROR_TIMEOUT,
        FACILITY_NTWIN32, INVALID_HANDLE_VALUE, NTSTATUS, STATUS_PENDING, STATUS_SUCCESS,
    },
    Networking::WinSock::{WSACleanup, WSAStartup, WSADATA},
    Storage::FileSystem::SetFileCompletionNotificationModes,
    System::{
        SystemServices::ERROR_SEVERITY_ERROR,
        Threading::INFINITE,
        WindowsProgramming::{FILE_SKIP_COMPLETION_PORT_ON_SUCCESS, FILE_SKIP_SET_EVENT_ON_HANDLE},
        IO::{
            CreateIoCompletionPort, GetQueuedCompletionStatusEx, PostQueuedCompletionStatus,
            OVERLAPPED, OVERLAPPED_ENTRY,
        },
    },
};

use crate::{syscall, AsyncifyPool, Entry, OutEntries, ProactorBuilder};

pub(crate) mod op;

pub(crate) use windows_sys::Win32::Networking::WinSock::{
    socklen_t, SOCKADDR_STORAGE as sockaddr_storage,
};

/// On windows, handle and socket are in the same size.
/// Both of them could be attached to an IOCP.
/// Therefore, both could be seen as fd.
pub type RawFd = RawHandle;

/// Extracts raw fds.
pub trait AsRawFd {
    /// Extracts the raw fd.
    fn as_raw_fd(&self) -> RawFd;
}

/// Construct IO objects from raw fds.
pub trait FromRawFd {
    /// Constructs an IO object from the specified raw fd.
    ///
    /// # Safety
    ///
    /// The `fd` passed in must:
    ///   - be a valid open handle or socket,
    ///   - be opened with `FILE_FLAG_OVERLAPPED` if it's a file handle,
    ///   - have not been attached to a driver.
    unsafe fn from_raw_fd(fd: RawFd) -> Self;
}

/// Consumes an object and acquire ownership of its raw fd.
pub trait IntoRawFd {
    /// Consumes this object, returning the raw underlying fd.
    fn into_raw_fd(self) -> RawFd;
}

impl AsRawFd for std::fs::File {
    fn as_raw_fd(&self) -> RawFd {
        self.as_raw_handle()
    }
}

impl AsRawFd for socket2::Socket {
    fn as_raw_fd(&self) -> RawFd {
        self.as_raw_socket() as _
    }
}

impl FromRawFd for std::fs::File {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self::from_raw_handle(fd)
    }
}

impl FromRawFd for socket2::Socket {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self::from_raw_socket(fd as _)
    }
}

impl IntoRawFd for std::fs::File {
    fn into_raw_fd(self) -> RawFd {
        self.into_raw_handle()
    }
}

impl IntoRawFd for socket2::Socket {
    fn into_raw_fd(self) -> RawFd {
        self.into_raw_socket() as _
    }
}

struct DriverEntry {
    queue: Mutex<VecDeque<Entry>>,
    event: Condvar,
}

impl DriverEntry {
    pub fn new(capacity: usize) -> Self {
        Self {
            queue: Mutex::new(VecDeque::with_capacity(capacity)),
            event: Condvar::new(),
        }
    }

    pub fn push(&self, entry: Entry) {
        self.queue.lock().unwrap().push_back(entry);
        self.event.notify_all();
    }

    pub fn wait(&self, timeout: Option<Duration>) -> io::Result<MutexGuard<VecDeque<Entry>>> {
        let guard = self.queue.lock().unwrap();
        if guard.is_empty() {
            if let Some(timeout) = timeout {
                let (guard, res) = self.event.wait_timeout(guard, timeout).unwrap();
                if res.timed_out() {
                    Err(io::Error::from_raw_os_error(ERROR_TIMEOUT as _))
                } else {
                    Ok(guard)
                }
            } else {
                Ok(self.event.wait(guard).unwrap())
            }
        } else {
            Ok(guard)
        }
    }
}

struct CompletionPort {
    port: OwnedHandle,
    drivers: SkipMap<usize, Arc<DriverEntry>>,
}

impl CompletionPort {
    pub fn new() -> io::Result<Self> {
        let port = syscall!(BOOL, CreateIoCompletionPort(INVALID_HANDLE_VALUE, 0, 0, 1))?;
        trace!("new iocp handle: {port}");
        let port = unsafe { OwnedHandle::from_raw_handle(port as _) };
        Ok(Self {
            port,
            drivers: SkipMap::new(),
        })
    }

    pub fn register(&self, driver: usize, capacity: usize) -> Arc<DriverEntry> {
        let driver_entry = Arc::new(DriverEntry::new(capacity));
        self.drivers.insert(driver, driver_entry.clone());
        driver_entry
    }

    pub fn attach(&self, fd: RawFd) -> io::Result<()> {
        syscall!(
            BOOL,
            CreateIoCompletionPort(fd as _, self.port.as_raw_handle() as _, 0, 0)
        )?;
        syscall!(
            BOOL,
            SetFileCompletionNotificationModes(
                fd as _,
                (FILE_SKIP_COMPLETION_PORT_ON_SUCCESS | FILE_SKIP_SET_EVENT_ON_HANDLE) as _
            )
        )?;
        Ok(())
    }

    pub fn post<T: ?Sized>(
        &self,
        res: io::Result<usize>,
        optr: *mut Overlapped<T>,
    ) -> io::Result<()> {
        if let Err(e) = &res {
            let code = e.raw_os_error().unwrap_or(ERROR_BAD_COMMAND as _);
            unsafe { &mut *optr }.base.Internal = ntstatus_from_win32(code) as _;
        }
        // We have to use CompletionKey to transfer the result because it is large
        // enough. It is OK because we set it to zero when attaching handles to IOCP.
        syscall!(
            BOOL,
            PostQueuedCompletionStatus(
                self.port.as_raw_handle() as _,
                0,
                res.unwrap_or_default(),
                optr.cast()
            )
        )?;
        Ok(())
    }

    pub fn push(&self, driver: usize, entry: Entry) {
        self.drivers
            .get(&driver)
            .expect("driver should register first")
            .value()
            .push(entry)
    }
}

impl AsRawHandle for CompletionPort {
    fn as_raw_handle(&self) -> RawHandle {
        self.port.as_raw_handle()
    }
}

static IOCP_PORT: OnceLock<CompletionPort> = OnceLock::new();

#[inline]
fn iocp_port() -> io::Result<&'static CompletionPort> {
    IOCP_PORT.get_or_try_init(CompletionPort::new)
}

fn iocp_start() -> io::Result<()> {
    const DEFAULT_CAPACITY: usize = 1024;

    let port = iocp_port()?;
    std::thread::spawn(move || {
        let mut entries = ArrayVec::<OVERLAPPED_ENTRY, { DEFAULT_CAPACITY }>::new();
        loop {
            let mut recv_count = 0;
            syscall!(
                BOOL,
                GetQueuedCompletionStatusEx(
                    port.as_raw_handle() as _,
                    entries.as_mut_ptr(),
                    DEFAULT_CAPACITY as _,
                    &mut recv_count,
                    INFINITE,
                    0
                )
            )?;
            trace!("recv_count: {recv_count}");
            unsafe { entries.set_len(recv_count as _) };

            for entry in entries.drain(..) {
                let transferred = entry.dwNumberOfBytesTransferred;
                trace!("entry transferred: {transferred}");
                // Any thin pointer is OK because we don't use the type of opcode.
                let overlapped_ptr: *mut Overlapped<()> = entry.lpOverlapped.cast();
                let overlapped = unsafe { &*overlapped_ptr };
                let res = if matches!(
                    overlapped.base.Internal as NTSTATUS,
                    STATUS_SUCCESS | STATUS_PENDING
                ) {
                    if entry.lpCompletionKey != 0 {
                        Ok(entry.lpCompletionKey)
                    } else {
                        Ok(transferred as _)
                    }
                } else {
                    let error = unsafe { RtlNtStatusToDosError(overlapped.base.Internal as _) };
                    match error {
                        ERROR_IO_INCOMPLETE | ERROR_HANDLE_EOF | ERROR_NO_DATA => Ok(0),
                        _ => Err(io::Error::from_raw_os_error(error as _)),
                    }
                };

                port.push(overlapped.driver, Entry::new(overlapped.user_data, res));
            }
        }
        #[allow(unreachable_code)]
        io::Result::Ok(())
    });
    Ok(())
}

/// Abstraction of IOCP operations.
pub trait OpCode {
    /// Determines that the operation is really overlapped defined by Windows
    /// API. If not, the driver will try to operate it in another thread.
    fn is_overlapped(&self) -> bool {
        true
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

fn ntstatus_from_win32(x: i32) -> NTSTATUS {
    if x <= 0 {
        x
    } else {
        ((x) & 0x0000FFFF) | (FACILITY_NTWIN32 << 16) as NTSTATUS | ERROR_SEVERITY_ERROR as NTSTATUS
    }
}

static DRIVER_COUNTER: AtomicUsize = AtomicUsize::new(0);
static IOCP_INIT_ONCE: OnceLock<()> = OnceLock::new();

/// Low-level driver of IOCP.
pub(crate) struct Driver {
    id: usize,
    driver_entry: Arc<DriverEntry>,
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

        IOCP_INIT_ONCE.get_or_try_init(iocp_start)?;

        let id = DRIVER_COUNTER.fetch_add(1, Ordering::AcqRel);
        let driver_entry = iocp_port()?.register(id, builder.capacity as _);
        Ok(Self {
            id,
            driver_entry,
            cancelled: HashSet::default(),
            pool: builder.create_or_get_thread_pool(),
            notify_overlapped: Arc::new(Overlapped::new(id, Self::NOTIFY, ())),
        })
    }

    pub fn create_op<T: OpCode + 'static>(&self, user_data: usize, op: T) -> RawOp {
        RawOp::new(self.id, user_data, op)
    }

    pub fn attach(&mut self, fd: RawFd) -> io::Result<()> {
        iocp_port()?.attach(fd)
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
            if op_pin.is_overlapped() {
                unsafe { op_pin.operate(optr.cast()) }
            } else if self.push_blocking(op)? {
                Poll::Pending
            } else {
                Poll::Ready(Err(io::Error::from_raw_os_error(ERROR_BUSY as _)))
            }
        }
    }

    fn push_blocking(&mut self, op: &mut RawOp) -> io::Result<bool> {
        // Safety: the RawOp is not released before the operation returns.
        struct SendWrapper<T>(T);
        unsafe impl<T> Send for SendWrapper<T> {}

        let optr = SendWrapper(NonNull::from(op));
        let port = iocp_port()?;
        Ok(self
            .pool
            .dispatch(move || {
                #[allow(clippy::redundant_locals)]
                let mut optr = optr;
                // Safety: the pointer is created from a reference.
                let op = unsafe { optr.0.as_mut() };
                let optr = op.as_mut_ptr();
                let op = op.as_op_pin();
                let res = unsafe { op.operate(optr.cast()) };
                let res = match res {
                    Poll::Pending => unreachable!("this operation is not overlapped"),
                    Poll::Ready(res) => res,
                };
                port.post(res, optr).ok();
            })
            .is_ok())
    }

    fn create_entry(entry: Entry, cancelled: &mut HashSet<usize>) -> Option<Entry> {
        let user_data = entry.user_data();
        if user_data != Self::NOTIFY {
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

        let mut completed_entries = self.driver_entry.wait(timeout)?;
        entries.extend(
            std::iter::from_fn(|| completed_entries.pop_front())
                .filter_map(|e| Self::create_entry(e, &mut self.cancelled)),
        );

        Ok(())
    }

    pub fn handle(&self) -> io::Result<NotifyHandle> {
        Ok(NotifyHandle::new(self.notify_overlapped.clone()))
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        syscall!(SOCKET, WSACleanup()).ok();
    }
}

/// A notify handle to the inner driver.
pub struct NotifyHandle {
    overlapped: Arc<Overlapped<()>>,
}

impl NotifyHandle {
    fn new(overlapped: Arc<Overlapped<()>>) -> Self {
        Self { overlapped }
    }

    /// Notify the inner driver.
    pub fn notify(&self) -> io::Result<()> {
        iocp_port()?.post(
            Ok(0),
            self.overlapped.as_ref() as *const _ as *mut Overlapped<()> as _,
        )
    }
}

/// The overlapped struct we actually used for IOCP.
#[repr(C)]
pub struct Overlapped<T: ?Sized> {
    /// The base [`OVERLAPPED`].
    pub base: OVERLAPPED,
    /// The unique ID of created driver.
    pub driver: usize,
    /// The registered user defined data.
    pub user_data: usize,
    /// The opcode.
    /// The user should guarantee the type is correct.
    pub op: T,
}

impl<T> Overlapped<T> {
    pub(crate) fn new(driver: usize, user_data: usize, op: T) -> Self {
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
    result: Option<io::Result<usize>>,
}

impl RawOp {
    pub(crate) fn new(driver: usize, user_data: usize, op: impl OpCode + 'static) -> Self {
        let op = Overlapped::new(driver, user_data, op);
        let op = Box::new(op) as Box<Overlapped<dyn OpCode>>;
        Self {
            op: unsafe { NonNull::new_unchecked(Box::into_raw(op)) },
            cancelled: false,
            result: None,
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
        self.result = Some(res);
        self.cancelled
    }

    pub fn has_result(&self) -> bool {
        self.result.is_some()
    }

    /// # Safety
    /// The caller should ensure the correct type.
    ///
    /// # Panics
    /// This function will panic if the result has not been set.
    pub unsafe fn into_inner<T: OpCode>(self) -> BufResult<usize, T> {
        let mut this = ManuallyDrop::new(self);
        let overlapped: Box<Overlapped<T>> = Box::from_raw(this.op.cast().as_ptr());
        BufResult(this.result.take().unwrap(), overlapped.op)
    }
}

impl Drop for RawOp {
    fn drop(&mut self) {
        if self.has_result() {
            let _ = unsafe { Box::from_raw(self.op.as_ptr()) };
        }
    }
}
