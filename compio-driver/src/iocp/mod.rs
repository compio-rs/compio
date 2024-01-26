use std::{
    collections::HashSet,
    io,
    mem::ManuallyDrop,
    os::windows::prelude::{
        AsRawHandle, AsRawSocket, FromRawHandle, FromRawSocket, IntoRawHandle, IntoRawSocket,
        OwnedHandle, RawHandle,
    },
    pin::Pin,
    ptr::{null_mut, NonNull},
    sync::Arc,
    task::Poll,
    time::Duration,
};

use compio_buf::{arrayvec::ArrayVec, BufResult};
use compio_log::{instrument, trace};
use slab::Slab;
use windows_sys::Win32::{
    Foundation::{
        RtlNtStatusToDosError, ERROR_BAD_COMMAND, ERROR_BUSY, ERROR_HANDLE_EOF,
        ERROR_IO_INCOMPLETE, ERROR_NO_DATA, ERROR_OPERATION_ABORTED, FACILITY_NTWIN32,
        INVALID_HANDLE_VALUE, NTSTATUS, STATUS_PENDING, STATUS_SUCCESS,
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
    /// Constructs a new IO object from the specified raw fd.
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

/// Low-level driver of IOCP.
pub(crate) struct Driver {
    // IOCP handle could not be duplicated.
    port: Arc<OwnedHandle>,
    cancelled: HashSet<usize>,
    pool: AsyncifyPool,
}

impl Driver {
    const DEFAULT_CAPACITY: usize = 1024;
    const NOTIFY: usize = usize::MAX;

    pub fn new(builder: &ProactorBuilder) -> io::Result<Self> {
        instrument!(compio_log::Level::TRACE, "new", ?builder);
        let mut data: WSADATA = unsafe { std::mem::zeroed() };
        syscall!(SOCKET, WSAStartup(0x202, &mut data))?;

        let port = syscall!(BOOL, CreateIoCompletionPort(INVALID_HANDLE_VALUE, 0, 0, 0))?;
        trace!("new iocp driver at port: {port}");
        let port = unsafe { OwnedHandle::from_raw_handle(port as _) };
        Ok(Self {
            port: Arc::new(port),
            cancelled: HashSet::default(),
            pool: builder.create_or_get_thread_pool(),
        })
    }

    #[inline]
    fn poll_impl<const N: usize>(
        &mut self,
        timeout: Option<Duration>,
        iocp_entries: &mut ArrayVec<OVERLAPPED_ENTRY, N>,
    ) -> io::Result<()> {
        instrument!(compio_log::Level::TRACE, "poll_impl", ?timeout);
        let mut recv_count = 0;
        let timeout = match timeout {
            Some(timeout) => timeout.as_millis() as u32,
            None => INFINITE,
        };
        syscall!(
            BOOL,
            GetQueuedCompletionStatusEx(
                self.port.as_raw_handle() as _,
                iocp_entries.as_mut_ptr(),
                N as _,
                &mut recv_count,
                timeout,
                0,
            )
        )?;
        trace!("recv_count: {recv_count}");
        unsafe {
            iocp_entries.set_len(recv_count as _);
        }
        Ok(())
    }

    fn create_entry(&mut self, iocp_entry: OVERLAPPED_ENTRY) -> Option<Entry> {
        if iocp_entry.lpOverlapped.is_null() {
            // This entry is posted by `post_driver_nop`.
            let user_data = iocp_entry.lpCompletionKey;
            trace!("entry {user_data} is posted by post_driver_nop");
            if user_data != Self::NOTIFY {
                let result = if self.cancelled.remove(&user_data) {
                    Err(io::Error::from_raw_os_error(ERROR_OPERATION_ABORTED as _))
                } else {
                    Ok(0)
                };
                Some(Entry::new(user_data, result))
            } else {
                None
            }
        } else {
            let transferred = iocp_entry.dwNumberOfBytesTransferred;
            // Any thin pointer is OK because we don't use the type of opcode.
            trace!("entry transferred: {transferred}");
            let overlapped_ptr: *mut Overlapped<()> = iocp_entry.lpOverlapped.cast();
            let overlapped = unsafe { &*overlapped_ptr };
            let res = if matches!(
                overlapped.base.Internal as NTSTATUS,
                STATUS_SUCCESS | STATUS_PENDING
            ) {
                Ok(transferred as _)
            } else {
                let error = unsafe { RtlNtStatusToDosError(overlapped.base.Internal as _) };
                match error {
                    ERROR_IO_INCOMPLETE | ERROR_HANDLE_EOF | ERROR_NO_DATA => Ok(0),
                    _ => Err(io::Error::from_raw_os_error(error as _)),
                }
            };
            Some(Entry::new(overlapped.user_data, res))
        }
    }

    pub fn attach(&mut self, fd: RawFd) -> io::Result<()> {
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
            } else if self.push_blocking(op) {
                Poll::Pending
            } else {
                Poll::Ready(Err(io::Error::from_raw_os_error(ERROR_BUSY as _)))
            }
        }
    }

    fn push_blocking(&mut self, op: &mut RawOp) -> bool {
        // Safety: the RawOp is not released before the operation returns.
        struct SendWrapper<T>(T);
        unsafe impl<T> Send for SendWrapper<T> {}

        let optr = SendWrapper(NonNull::from(op));
        let handle = self.as_raw_fd() as _;
        self.pool
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
                if let Err(e) = &res {
                    let code = e.raw_os_error().unwrap_or(ERROR_BAD_COMMAND as _);
                    unsafe { &mut *optr }.base.Internal = ntstatus_from_win32(code) as _;
                }
                syscall!(
                    BOOL,
                    PostQueuedCompletionStatus(
                        handle,
                        res.unwrap_or_default() as _,
                        0,
                        optr.cast()
                    )
                )
                .ok();
            })
            .is_ok()
    }

    pub unsafe fn poll(
        &mut self,
        timeout: Option<Duration>,
        mut entries: OutEntries<impl Extend<usize>>,
    ) -> io::Result<()> {
        instrument!(compio_log::Level::TRACE, "poll", ?timeout);
        // Prevent stack growth.
        let mut iocp_entries = ArrayVec::<OVERLAPPED_ENTRY, { Self::DEFAULT_CAPACITY }>::new();
        self.poll_impl(timeout, &mut iocp_entries)?;
        entries.extend(iocp_entries.drain(..).filter_map(|e| self.create_entry(e)));

        // See if there are remaining entries.
        loop {
            match self.poll_impl(Some(Duration::ZERO), &mut iocp_entries) {
                Ok(()) => {
                    entries.extend(iocp_entries.drain(..).filter_map(|e| self.create_entry(e)));
                }
                Err(e) => match e.kind() {
                    io::ErrorKind::TimedOut => {
                        trace!("poll timeout");
                        break;
                    }
                    _ => return Err(e),
                },
            }
        }

        Ok(())
    }

    pub fn handle(&self) -> io::Result<NotifyHandle> {
        self.handle_for(Self::NOTIFY)
    }

    pub fn handle_for(&self, user_data: usize) -> io::Result<NotifyHandle> {
        Ok(NotifyHandle::new(user_data, self.port.clone()))
    }
}

impl AsRawFd for Driver {
    fn as_raw_fd(&self) -> RawFd {
        self.port.as_raw_handle()
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        syscall!(SOCKET, WSACleanup()).ok();
    }
}

/// A notify handle to the inner driver.
pub struct NotifyHandle {
    user_data: usize,
    handle: Arc<OwnedHandle>,
}

unsafe impl Send for NotifyHandle {}
unsafe impl Sync for NotifyHandle {}

impl NotifyHandle {
    fn new(user_data: usize, handle: Arc<OwnedHandle>) -> Self {
        Self { user_data, handle }
    }

    /// Notify the inner driver.
    pub fn notify(&self) -> io::Result<()> {
        syscall!(
            BOOL,
            PostQueuedCompletionStatus(
                self.handle.as_raw_handle() as _,
                0,
                self.user_data,
                null_mut()
            )
        )?;
        Ok(())
    }
}

/// The overlapped struct we actually used for IOCP.
#[repr(C)]
pub struct Overlapped<T: ?Sized> {
    /// The base [`OVERLAPPED`].
    pub base: OVERLAPPED,
    /// The registered user defined data.
    pub user_data: usize,
    /// The opcode.
    /// The user should guarantee the type is correct.
    pub op: T,
}

impl<T> Overlapped<T> {
    pub(crate) fn new(user_data: usize, op: T) -> Self {
        Self {
            base: unsafe { std::mem::zeroed() },
            user_data,
            op,
        }
    }
}

pub(crate) struct RawOp {
    op: NonNull<Overlapped<dyn OpCode>>,
    // The two flags here are manual reference counting. The driver holds the strong ref until it
    // completes; the runtime holds the strong ref until the future is dropped.
    cancelled: bool,
    result: Option<io::Result<usize>>,
}

impl RawOp {
    pub(crate) fn new(user_data: usize, op: impl OpCode + 'static) -> Self {
        let op = Overlapped::new(user_data, op);
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
