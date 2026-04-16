use windows_sys::Win32::{
    Foundation::CloseHandle, Storage::FileSystem::FlushFileBuffers, System::IO::OVERLAPPED,
};

use crate::{
    OpCode, OpType,
    sys::{op::*, prelude::*},
};

unsafe impl OpCode for CloseFile {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn op_type(&self, _: &Self::Control) -> OpType {
        OpType::Blocking
    }

    unsafe fn operate(&mut self, _: &mut (), _optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(
            syscall!(BOOL, CloseHandle(self.fd.as_fd().as_raw_fd()))? as _,
        ))
    }
}

unsafe impl<S: AsFd> OpCode for Sync<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn op_type(&self, _: &Self::Control) -> OpType {
        OpType::Blocking
    }

    unsafe fn operate(&mut self, _: &mut (), _optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(
            syscall!(BOOL, FlushFileBuffers(self.fd.as_fd().as_raw_fd()))? as _,
        ))
    }
}
