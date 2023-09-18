use std::{
    collections::HashSet,
    io,
    marker::PhantomData,
    os::windows::{
        prelude::{
            AsRawHandle, AsRawSocket, FromRawHandle, FromRawSocket, IntoRawHandle, IntoRawSocket,
            OwnedHandle, RawHandle,
        },
    },
    task::Poll,
    time::Duration,
};

use arrayvec::ArrayVec;
use windows_sys::Win32::{
    Foundation::{
        RtlNtStatusToDosError, ERROR_HANDLE_EOF, ERROR_IO_INCOMPLETE, ERROR_NO_DATA,
        FACILITY_NTWIN32, INVALID_HANDLE_VALUE, NTSTATUS, STATUS_PENDING, STATUS_SUCCESS,
    },
    System::{
        SystemServices::ERROR_SEVERITY_ERROR,
        Threading::INFINITE,
        IO::{
            CreateIoCompletionPort, GetQueuedCompletionStatusEx, PostQueuedCompletionStatus,
            OVERLAPPED, OVERLAPPED_ENTRY,
        },
    },
};

use crate::{
    driver::{Entry, Operation, Poller},
    syscall,
};

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
    ///   - have been attached to a driver.
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
    /// # Safety
    ///
    /// `self` attributes must be Unpin to ensure safe operation.
    unsafe fn operate(&mut self, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>>;
}

/// Low-level driver of IOCP.
pub struct Driver<'arena> {
    port: OwnedHandle,
    cancelled: HashSet<usize>,
    _lifetime: PhantomData<&'arena ()>,
}

impl<'arena> Driver<'arena> {
    const DEFAULT_CAPACITY: usize = 1024;

    /// Create a new IOCP.
    pub fn new() -> io::Result<Self> {
        Self::with_entries(Self::DEFAULT_CAPACITY as _)
    }

    /// The same as [`Driver::new`].
    pub fn with_entries(_entries: u32) -> io::Result<Self> {
        let port = syscall!(BOOL, CreateIoCompletionPort(INVALID_HANDLE_VALUE, 0, 0, 0))?;
        let port = unsafe { OwnedHandle::from_raw_handle(port as _) };
        Ok(Self {
            port,
            cancelled: HashSet::default(),
            _lifetime: PhantomData,
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
        let transferred = iocp_entry.dwNumberOfBytesTransferred;
        let overlapped_ptr = iocp_entry.lpOverlapped;
        let overlapped = unsafe { Box::from_raw(overlapped_ptr.cast::<Overlapped>()) };
        if self.cancelled.remove(&overlapped.user_data) {
            return None;
        }
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

/// # Safety
///
/// * The handle should be valid.
/// * The overlapped_ptr should be non-null.
unsafe fn post_driver_raw(
    handle: RawFd,
    result: io::Result<usize>,
    overlapped_ptr: *mut OVERLAPPED,
) -> io::Result<()> {
    if let Err(e) = &result {
        (*overlapped_ptr).Internal = ntstatus_from_win32(e.raw_os_error().unwrap_or_default()) as _;
    }
    syscall!(
        BOOL,
        PostQueuedCompletionStatus(
            handle as _,
            result.unwrap_or_default() as _,
            0,
            overlapped_ptr,
        )
    )?;
    Ok(())
}

#[cfg(feature = "event")]
pub(crate) fn post_driver(
    handle: RawFd,
    user_data: usize,
    result: io::Result<usize>,
) -> io::Result<()> {
    let overlapped = Box::new(Overlapped::new(user_data));
    let overlapped_ptr = Box::into_raw(overlapped);
    unsafe { post_driver_raw(handle, result, overlapped_ptr.cast()) }
}

fn ntstatus_from_win32(x: i32) -> NTSTATUS {
    if x <= 0 {
        x
    } else {
        (x & 0x0000FFFF) | (FACILITY_NTWIN32 << 16) as NTSTATUS | ERROR_SEVERITY_ERROR as NTSTATUS
    }
}

impl<'arena> Poller<'arena> for Driver<'arena> {
    fn attach(&mut self, fd: RawFd) -> io::Result<()> {
        syscall!(
            BOOL,
            CreateIoCompletionPort(fd as _, self.port.as_raw_handle() as _, 0, 0)
        )?;
        Ok(())
    }

    fn cancel(&mut self, user_data: usize) {
        self.cancelled.insert(user_data);
    }

    unsafe fn poll(
        &mut self,
        timeout: Option<Duration>,
        ops: &mut impl Iterator<Item = Operation<'arena>>,
        entries: &mut impl Extend<Entry>,
    ) -> io::Result<()> {
        for mut operation in ops {
            if !self.cancelled.remove(&operation.user_data()) {
                let overlapped = Box::new(Overlapped::new(operation.user_data()));
                let overlapped_ptr = Box::into_raw(overlapped);
                // we require Unpin buffers - so no need to pin
                let op = operation.opcode();
                let result = op.operate(overlapped_ptr.cast());
                if let Poll::Ready(result) = result {
                    post_driver_raw(self.port.as_raw_handle(), result, overlapped_ptr.cast())?;
                }
            }
        }

        // Prevent stack growth.
        const CAP: usize = Driver::DEFAULT_CAPACITY;
        let mut iocp_entries = ArrayVec::<OVERLAPPED_ENTRY, CAP>::new();
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

impl AsRawFd for Driver<'_> {
    fn as_raw_fd(&self) -> RawFd {
        self.port.as_raw_handle()
    }
}

#[repr(C)]
struct Overlapped {
    #[allow(dead_code)]
    pub base: OVERLAPPED,
    pub user_data: usize,
}

impl Overlapped {
    pub fn new(user_data: usize) -> Self {
        Self {
            base: unsafe { std::mem::zeroed() },
            user_data,
        }
    }
}
