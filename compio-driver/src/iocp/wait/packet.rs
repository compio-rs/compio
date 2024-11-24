use std::{
    ffi::c_void,
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    ptr::null_mut,
};

use windows_sys::Win32::Foundation::{
    BOOLEAN, GENERIC_READ, GENERIC_WRITE, HANDLE, NTSTATUS, RtlNtStatusToDosError, STATUS_PENDING,
    STATUS_SUCCESS,
};

use crate::{Key, OpCode, RawFd, sys::cp};

extern "system" {
    fn NtCreateWaitCompletionPacket(
        WaitCompletionPacketHandle: *mut HANDLE,
        DesiredAccess: u32,
        ObjectAttributes: *mut c_void,
    ) -> NTSTATUS;

    fn NtAssociateWaitCompletionPacket(
        WaitCompletionPacketHandle: HANDLE,
        IoCompletionHandle: HANDLE,
        TargetObjectHandle: HANDLE,
        KeyContext: *mut c_void,
        ApcContext: *mut c_void,
        IoStatus: NTSTATUS,
        IoStatusInformation: usize,
        AlreadySignaled: *mut BOOLEAN,
    ) -> NTSTATUS;

    fn NtCancelWaitCompletionPacket(
        WaitCompletionPacketHandle: HANDLE,
        RemoveSignaledPacket: BOOLEAN,
    ) -> NTSTATUS;
}

pub struct Wait {
    handle: OwnedHandle,
    cancelled: bool,
}

fn check_status(status: NTSTATUS) -> io::Result<()> {
    if status == STATUS_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(unsafe {
            RtlNtStatusToDosError(status) as _
        }))
    }
}

impl Wait {
    pub fn new(port: &cp::Port, event: RawFd, op: &mut Key<dyn OpCode>) -> io::Result<Self> {
        let mut handle = 0;
        check_status(unsafe {
            NtCreateWaitCompletionPacket(&mut handle, GENERIC_READ | GENERIC_WRITE, null_mut())
        })?;
        let handle = unsafe { OwnedHandle::from_raw_handle(handle as _) };
        check_status(unsafe {
            NtAssociateWaitCompletionPacket(
                handle.as_raw_handle() as _,
                port.as_raw_handle() as _,
                event,
                null_mut(),
                op.as_mut_ptr().cast(),
                STATUS_SUCCESS,
                0,
                null_mut(),
            )
        })?;
        Ok(Self {
            handle,
            cancelled: false,
        })
    }

    pub fn cancel(&mut self) -> io::Result<()> {
        let res = unsafe { NtCancelWaitCompletionPacket(self.handle.as_raw_handle() as _, 0) };
        self.cancelled = res != STATUS_PENDING;
        check_status(res)
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }
}
