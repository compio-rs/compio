use std::{
    collections::HashSet,
    io,
    mem::ManuallyDrop,
    os::windows::prelude::{
        AsRawHandle, AsRawSocket, FromRawHandle, FromRawSocket, IntoRawHandle, IntoRawSocket,
        OwnedHandle, RawHandle,
    },
    pin::Pin,
    ptr::NonNull,
    task::Poll,
    time::Duration,
};

use compio_buf::arrayvec::ArrayVec;
use slab::Slab;
use windows_sys::Win32::{
    Foundation::{
        RtlNtStatusToDosError, ERROR_HANDLE_EOF, ERROR_IO_INCOMPLETE, ERROR_NO_DATA,
        ERROR_OPERATION_ABORTED, INVALID_HANDLE_VALUE, NTSTATUS, STATUS_PENDING, STATUS_SUCCESS,
    },
    Storage::FileSystem::SetFileCompletionNotificationModes,
    System::{
        Threading::INFINITE,
        WindowsProgramming::{FILE_SKIP_COMPLETION_PORT_ON_SUCCESS, FILE_SKIP_SET_EVENT_ON_HANDLE},
        IO::{CreateIoCompletionPort, GetQueuedCompletionStatusEx, OVERLAPPED, OVERLAPPED_ENTRY},
    },
};

use crate::{syscall, Entry};

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

/// Contruct IO objects from raw fds.
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
    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()>;
}

/// Low-level driver of IOCP.
pub(crate) struct Driver {
    port: OwnedHandle,
    cancelled: HashSet<usize>,
}

impl Driver {
    const DEFAULT_CAPACITY: usize = 1024;

    pub fn new(_entries: u32) -> io::Result<Self> {
        let port = syscall!(BOOL, CreateIoCompletionPort(INVALID_HANDLE_VALUE, 0, 0, 0))?;
        let port = unsafe { OwnedHandle::from_raw_handle(port as _) };
        Ok(Self {
            port,
            cancelled: HashSet::default(),
        })
    }

    #[inline]
    fn poll_impl<const N: usize>(
        &mut self,
        timeout: Option<Duration>,
        iocp_entries: &mut ArrayVec<OVERLAPPED_ENTRY, N>,
    ) -> io::Result<()> {
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
        unsafe {
            iocp_entries.set_len(recv_count as _);
        }
        Ok(())
    }

    fn create_entry(&mut self, iocp_entry: OVERLAPPED_ENTRY) -> Option<Entry> {
        if iocp_entry.lpOverlapped.is_null() {
            // This entry is posted by `post_driver_nop`.
            let user_data = iocp_entry.lpCompletionKey;
            let result = if self.cancelled.remove(&user_data) {
                Err(io::Error::from_raw_os_error(ERROR_OPERATION_ABORTED as _))
            } else {
                Ok(0)
            };
            Some(Entry::new(user_data, result))
        } else {
            let transferred = iocp_entry.dwNumberOfBytesTransferred;
            // Any thin pointer is OK because we don't use the type of opcode.
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
        self.cancelled.insert(user_data);
        if let Some(op) = registry.get_mut(user_data) {
            let overlapped_ptr = op.as_mut_ptr();
            let op = op.as_op_pin();
            // It's OK to fail to cancel.
            unsafe { op.cancel(overlapped_ptr.cast()) }.ok();
        }
    }

    pub fn push(&mut self, user_data: usize, op: &mut RawOp) -> Poll<io::Result<usize>> {
        if self.cancelled.remove(&user_data) {
            Poll::Ready(Err(io::Error::from_raw_os_error(
                ERROR_OPERATION_ABORTED as _,
            )))
        } else {
            let optr = op.as_mut_ptr();
            unsafe { op.as_op_pin().operate(optr.cast()) }
        }
    }

    pub unsafe fn poll(
        &mut self,
        timeout: Option<Duration>,
        entries: &mut impl Extend<Entry>,
        _registry: &mut Slab<RawOp>,
    ) -> io::Result<()> {
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
                    io::ErrorKind::TimedOut => break,
                    _ => return Err(e),
                },
            }
        }

        Ok(())
    }
}

impl AsRawFd for Driver {
    fn as_raw_fd(&self) -> RawFd {
        self.port.as_raw_handle()
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

#[doc(hidden)]
pub struct RawOp(NonNull<Overlapped<dyn OpCode>>);

impl RawOp {
    pub(crate) fn new(user_data: usize, op: impl OpCode + 'static) -> Self {
        let op = Overlapped::new(user_data, op);
        let op = Box::new(op) as Box<Overlapped<dyn OpCode>>;
        Self(unsafe { NonNull::new_unchecked(Box::into_raw(op)) })
    }

    pub fn as_op_pin(&mut self) -> Pin<&mut dyn OpCode> {
        unsafe { Pin::new_unchecked(&mut self.0.as_mut().op) }
    }

    pub fn as_mut_ptr(&mut self) -> *mut Overlapped<dyn OpCode> {
        self.0.as_ptr()
    }

    /// # Safety
    /// The caller should ensure the correct type.
    pub unsafe fn into_inner<T: OpCode>(self) -> T {
        let this = ManuallyDrop::new(self);
        let this: Box<Overlapped<T>> = Box::from_raw(this.0.cast().as_ptr());
        this.op
    }
}
