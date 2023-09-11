use std::{
    collections::{HashMap, HashSet, VecDeque},
    ffi::c_void,
    io,
    mem::MaybeUninit,
    os::windows::{
        io::HandleOrNull,
        prelude::{
            AsRawHandle, AsRawSocket, FromRawHandle, FromRawSocket, IntoRawHandle, IntoRawSocket,
            OwnedHandle, RawHandle,
        },
    },
    ptr::null,
    task::Poll,
    time::Duration,
};

use windows_sys::Win32::{
    Foundation::{
        RtlNtStatusToDosError, ERROR_HANDLE_EOF, ERROR_IO_INCOMPLETE, ERROR_NO_DATA,
        FACILITY_NTWIN32, INVALID_HANDLE_VALUE, NTSTATUS, STATUS_PENDING, STATUS_SUCCESS,
    },
    System::{
        SystemServices::ERROR_SEVERITY_ERROR,
        Threading::INFINITE,
        WindowsProgramming::{FILE_INFORMATION_CLASS, IO_STATUS_BLOCK},
        IO::{
            CreateIoCompletionPort, GetQueuedCompletionStatusEx, PostQueuedCompletionStatus,
            OVERLAPPED,
        },
    },
};

use crate::driver::{Entry, Poller};

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
    /// `self` must be alive until the operation completes.
    unsafe fn operate(&mut self, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>>;
}

/// Low-level driver of IOCP.
pub struct Driver {
    port: OwnedHandle,
    operations: VecDeque<(*mut dyn OpCode, Overlapped)>,
    submit_map: HashMap<usize, *mut OVERLAPPED>,
    cancelled: HashSet<*mut OVERLAPPED>,
}

impl Driver {
    const DEFAULT_CAPACITY: usize = 1024;

    /// Create a new IOCP.
    pub fn new() -> io::Result<Self> {
        let port = unsafe { CreateIoCompletionPort(INVALID_HANDLE_VALUE, 0, 0, 0) };
        let port = OwnedHandle::try_from(unsafe { HandleOrNull::from_raw_handle(port as _) })
            .map_err(|_| io::Error::last_os_error())?;
        Ok(Self {
            port,
            operations: VecDeque::with_capacity(Self::DEFAULT_CAPACITY),
            submit_map: HashMap::default(),
            cancelled: HashSet::default(),
        })
    }
}

fn post_driver_raw(
    handle: RawFd,
    result: io::Result<usize>,
    overlapped_ptr: *mut OVERLAPPED,
) -> io::Result<()> {
    if let Err(e) = &result {
        unsafe {
            (*overlapped_ptr).Internal =
                ntstatus_from_win32(e.raw_os_error().unwrap_or_default()) as _;
        }
    }
    let res = unsafe {
        PostQueuedCompletionStatus(
            handle as _,
            result.unwrap_or_default() as _,
            0,
            overlapped_ptr,
        )
    };
    if res == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(feature = "event")]
pub(crate) fn post_driver(
    handle: RawFd,
    user_data: usize,
    result: io::Result<usize>,
) -> io::Result<()> {
    let overlapped = Box::new(Overlapped::new(user_data));
    let overlapped_ptr = Box::into_raw(overlapped);
    post_driver_raw(handle, result, overlapped_ptr.cast())
}

fn detach_iocp(fd: RawFd) -> io::Result<()> {
    #[link(name = "ntdll")]
    extern "system" {
        fn NtSetInformationFile(
            FileHandle: usize,
            IoStatusBlock: *mut IO_STATUS_BLOCK,
            FileInformation: *const c_void,
            Length: u32,
            FileInformationClass: FILE_INFORMATION_CLASS,
        ) -> NTSTATUS;
    }
    #[allow(non_upper_case_globals)]
    const FileReplaceCompletionInformation: FILE_INFORMATION_CLASS = 61;
    #[repr(C)]
    #[allow(non_camel_case_types)]
    #[allow(non_snake_case)]
    struct FILE_COMPLETION_INFORMATION {
        Port: usize,
        Key: *const c_void,
    }

    let mut block = unsafe { std::mem::zeroed() };
    let info = FILE_COMPLETION_INFORMATION {
        Port: 0,
        Key: null(),
    };
    unsafe {
        NtSetInformationFile(
            fd as _,
            &mut block,
            &info as *const _ as _,
            std::mem::size_of_val(&info) as _,
            FileReplaceCompletionInformation,
        )
    };
    let res = unsafe { block.Anonymous.Status };
    if res != STATUS_SUCCESS {
        Err(io::Error::from_raw_os_error(unsafe {
            RtlNtStatusToDosError(res) as _
        }))
    } else {
        Ok(())
    }
}

fn ntstatus_from_win32(x: i32) -> NTSTATUS {
    if x <= 0 {
        x
    } else {
        (x & 0x0000FFFF) | (FACILITY_NTWIN32 << 16) as NTSTATUS | ERROR_SEVERITY_ERROR as NTSTATUS
    }
}

impl Poller for Driver {
    fn attach(&mut self, fd: RawFd) -> io::Result<()> {
        // Wine doesn't support the replacement.
        detach_iocp(fd).ok();
        let port = unsafe { CreateIoCompletionPort(fd as _, self.port.as_raw_handle() as _, 0, 0) };
        if port == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    unsafe fn push(
        &mut self,
        op: &mut (impl OpCode + 'static),
        user_data: usize,
    ) -> io::Result<()> {
        self.operations.push_back((op, Overlapped::new(user_data)));
        Ok(())
    }

    fn cancel(&mut self, user_data: usize) {
        if let Some(ptr) = self.submit_map.remove(&user_data) {
            // TODO: should we call CancelIoEx?
            self.cancelled.insert(ptr);
        }
    }

    fn poll(
        &mut self,
        timeout: Option<Duration>,
        entries: &mut [MaybeUninit<Entry>],
    ) -> io::Result<usize> {
        if entries.is_empty() {
            return Ok(0);
        }
        while let Some((op, overlapped)) = self.operations.pop_front() {
            let overlapped = Box::new(overlapped);
            let user_data = overlapped.user_data;
            let overlapped_ptr = Box::into_raw(overlapped);
            let result = unsafe { op.as_mut().unwrap().operate(overlapped_ptr.cast()) };
            if let Poll::Ready(result) = result {
                post_driver_raw(self.port.as_raw_handle(), result, overlapped_ptr.cast())?;
            } else {
                self.submit_map.insert(user_data, overlapped_ptr.cast());
            }
        }

        let mut iocp_entries = Vec::with_capacity(entries.len());
        let mut recv_count = 0;
        let timeout = match timeout {
            Some(timeout) => timeout.as_millis() as u32,
            None => INFINITE,
        };
        let res = unsafe {
            GetQueuedCompletionStatusEx(
                self.port.as_raw_handle() as _,
                iocp_entries.as_mut_ptr(),
                entries.len() as _,
                &mut recv_count,
                timeout,
                0,
            )
        };
        if res == 0 {
            return Err(io::Error::last_os_error());
        }
        unsafe {
            iocp_entries.set_len(recv_count as _);
        }
        let iocp_len = iocp_entries.len();
        debug_assert!(iocp_len <= entries.len());

        for (iocp_entry, entry) in iocp_entries.into_iter().zip(entries) {
            let transferred = iocp_entry.dwNumberOfBytesTransferred;
            let overlapped_ptr = iocp_entry.lpOverlapped;
            let overlapped = unsafe { Box::from_raw(overlapped_ptr.cast::<Overlapped>()) };
            if self.cancelled.remove(&overlapped_ptr) {
                continue;
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
            entry.write(Entry::new(overlapped.user_data, res));
        }
        Ok(iocp_len)
    }
}

impl AsRawFd for Driver {
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
