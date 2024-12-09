//! Completion Port
//!
//! This mod contains utilities of IOCP. It provides 2 working modes:
//! IOCP-per-thread, and IOCP-global.
//!
//! ## IOCP-per-thread
//! In `mod multi`. Each driver hosts a seperate port. If the port receives
//! entry that doesn't belong to the current port, it will try to repost it to
//! the correct port.
//!
//! ## IOCP-global
//! In `mod global`. A main port runs in a separate thread, and dispatches all
//! entries to the correct driver.

use std::{
    io,
    mem::MaybeUninit,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle},
    time::Duration,
};

use compio_log::*;
use windows_sys::Win32::{
    Foundation::{
        ERROR_BAD_COMMAND, ERROR_BROKEN_PIPE, ERROR_HANDLE_EOF, ERROR_IO_INCOMPLETE, ERROR_NO_DATA,
        ERROR_PIPE_CONNECTED, ERROR_PIPE_NOT_CONNECTED, FACILITY_NTWIN32, INVALID_HANDLE_VALUE,
        NTSTATUS, RtlNtStatusToDosError, STATUS_SUCCESS,
    },
    Storage::FileSystem::SetFileCompletionNotificationModes,
    System::{
        IO::{
            CreateIoCompletionPort, GetQueuedCompletionStatusEx, OVERLAPPED_ENTRY,
            PostQueuedCompletionStatus,
        },
        SystemServices::ERROR_SEVERITY_ERROR,
        Threading::INFINITE,
        WindowsProgramming::{FILE_SKIP_COMPLETION_PORT_ON_SUCCESS, FILE_SKIP_SET_EVENT_ON_HANDLE},
    },
};

use crate::{Entry, Overlapped, RawFd, syscall};

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
    pub const DEFAULT_CAPACITY: usize = 1024;

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

    pub fn post(&self, res: io::Result<usize>, optr: *mut Overlapped) -> io::Result<()> {
        if let Some(overlapped) = unsafe { optr.as_mut() } {
            match &res {
                Ok(transferred) => {
                    overlapped.base.Internal = STATUS_SUCCESS as _;
                    overlapped.base.InternalHigh = *transferred;
                }
                Err(e) => {
                    let code = e.raw_os_error().unwrap_or(ERROR_BAD_COMMAND as _);
                    overlapped.base.Internal = ntstatus_from_win32(code) as _;
                }
            }
        }
        self.post_raw(optr)
    }

    pub fn post_raw(&self, optr: *const Overlapped) -> io::Result<()> {
        syscall!(
            BOOL,
            PostQueuedCompletionStatus(self.port.as_raw_handle() as _, 0, 0, optr.cast())
        )?;
        Ok(())
    }

    pub fn poll_raw(
        &self,
        timeout: Option<Duration>,
        entries: &mut [MaybeUninit<OVERLAPPED_ENTRY>],
    ) -> io::Result<usize> {
        let mut recv_count = 0;
        let timeout = match timeout {
            Some(timeout) => timeout.as_millis() as u32,
            None => INFINITE,
        };
        syscall!(
            BOOL,
            GetQueuedCompletionStatusEx(
                self.port.as_raw_handle() as _,
                entries.as_mut_ptr().cast(),
                entries.len() as _,
                &mut recv_count,
                timeout,
                0
            )
        )?;
        trace!("recv_count: {recv_count}");

        Ok(recv_count as _)
    }

    // If current_driver is specified, any entry that doesn't belong the driver will
    // be reposted. The driver id will be used as IOCP handle.
    pub fn poll(
        &self,
        timeout: Option<Duration>,
        current_driver: Option<RawFd>,
    ) -> io::Result<impl Iterator<Item = Entry>> {
        let mut entries = Vec::with_capacity(Self::DEFAULT_CAPACITY);
        let len = self.poll_raw(timeout, entries.spare_capacity_mut())?;
        unsafe { entries.set_len(len) };
        Ok(entries.into_iter().filter_map(move |entry| {
            // Any thin pointer is OK because we don't use the type of opcode.
            let overlapped_ptr: *mut Overlapped = entry.lpOverlapped.cast();
            let overlapped = unsafe { &*overlapped_ptr };
            if let Some(current_driver) = current_driver {
                if overlapped.driver != current_driver {
                    // Repost the entry to correct port.
                    if let Err(_e) = syscall!(
                        BOOL,
                        PostQueuedCompletionStatus(
                            overlapped.driver as _,
                            entry.dwNumberOfBytesTransferred,
                            entry.lpCompletionKey,
                            entry.lpOverlapped,
                        )
                    ) {
                        error!(
                            "fail to repost entry ({}, {}, {:p}) to driver {:x}: {:?}",
                            entry.dwNumberOfBytesTransferred,
                            entry.lpCompletionKey,
                            entry.lpOverlapped,
                            overlapped.driver,
                            _e
                        );
                    }
                    return None;
                }
            }
            // TODO: *mut OVERLAPPED is *mut IO_STATUS_BLOCK internally, but
            // OVERLAPPED::Internal is not the same size as
            // IO_STATUS_BLOCK::Status.
            let status = overlapped.base.Internal as NTSTATUS;
            let res = if status >= 0 {
                Ok(overlapped.base.InternalHigh)
            } else {
                let error = unsafe { RtlNtStatusToDosError(status) };
                match error {
                    ERROR_IO_INCOMPLETE
                    | ERROR_HANDLE_EOF
                    | ERROR_BROKEN_PIPE
                    | ERROR_PIPE_CONNECTED
                    | ERROR_PIPE_NOT_CONNECTED
                    | ERROR_NO_DATA => Ok(0),
                    _ => Err(io::Error::from_raw_os_error(error as _)),
                }
            };
            Some(Entry::new(overlapped_ptr as usize, res))
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
