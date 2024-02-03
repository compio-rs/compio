use std::{
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle},
    time::Duration,
};

use compio_buf::arrayvec::ArrayVec;
use compio_log::*;
use windows_sys::Win32::{
    Foundation::{
        RtlNtStatusToDosError, ERROR_BAD_COMMAND, ERROR_HANDLE_EOF, ERROR_IO_INCOMPLETE,
        ERROR_NO_DATA, FACILITY_NTWIN32, INVALID_HANDLE_VALUE, NTSTATUS, STATUS_PENDING,
        STATUS_SUCCESS,
    },
    Storage::FileSystem::SetFileCompletionNotificationModes,
    System::{
        SystemServices::ERROR_SEVERITY_ERROR,
        Threading::INFINITE,
        WindowsProgramming::{FILE_SKIP_COMPLETION_PORT_ON_SUCCESS, FILE_SKIP_SET_EVENT_ON_HANDLE},
        IO::{
            CreateIoCompletionPort, GetQueuedCompletionStatusEx, PostQueuedCompletionStatus,
            OVERLAPPED_ENTRY,
        },
    },
};

use crate::{syscall, Entry, Overlapped, RawFd};

cfg_if::cfg_if! {
    if #[cfg(feature = "iocp-global")] {
        mod global;
        pub use global::*;
    } else {
        mod multi;
        pub use multi::*;
    }
}

struct CompletionPort {
    port: OwnedHandle,
}

impl CompletionPort {
    pub fn new() -> io::Result<Self> {
        let port = syscall!(BOOL, CreateIoCompletionPort(INVALID_HANDLE_VALUE, 0, 0, 1))?;
        trace!("new iocp handle: {port}");
        let port = unsafe { OwnedHandle::from_raw_handle(port as _) };
        Ok(Self { port })
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

    pub fn poll(
        &self,
        timeout: Option<Duration>,
    ) -> io::Result<impl Iterator<Item = (usize, Entry)>> {
        const DEFAULT_CAPACITY: usize = 1024;

        let mut entries = ArrayVec::<OVERLAPPED_ENTRY, { DEFAULT_CAPACITY }>::new();
        let mut recv_count = 0;
        let timeout = match timeout {
            Some(timeout) => timeout.as_millis() as u32,
            None => INFINITE,
        };
        syscall!(
            BOOL,
            GetQueuedCompletionStatusEx(
                self.port.as_raw_handle() as _,
                entries.as_mut_ptr(),
                DEFAULT_CAPACITY as _,
                &mut recv_count,
                timeout,
                0
            )
        )?;
        trace!("recv_count: {recv_count}");
        unsafe { entries.set_len(recv_count as _) };

        Ok(entries.into_iter().map(|entry| {
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
            (overlapped.driver, Entry::new(overlapped.user_data, res))
        }))
    }
}

impl AsRawHandle for CompletionPort {
    fn as_raw_handle(&self) -> RawHandle {
        self.port.as_raw_handle()
    }
}

#[inline]
fn ntstatus_from_win32(x: i32) -> NTSTATUS {
    if x <= 0 {
        x
    } else {
        ((x) & 0x0000FFFF) | (FACILITY_NTWIN32 << 16) as NTSTATUS | ERROR_SEVERITY_ERROR as NTSTATUS
    }
}
