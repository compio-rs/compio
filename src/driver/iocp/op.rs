use crate::{
    buf::{AsBuf, AsBufMut, IoBuf, IoBufMut},
    driver::OpCode,
    op::{ReadAt, WriteAt},
};
use std::{io, task::Poll};
use windows_sys::Win32::{
    Foundation::{
        GetLastError, ERROR_HANDLE_EOF, ERROR_IO_INCOMPLETE, ERROR_IO_PENDING, ERROR_NO_DATA,
        ERROR_PIPE_CONNECTED,
    },
    Storage::FileSystem::{ReadFile, WriteFile},
    System::IO::OVERLAPPED,
};

unsafe fn win32_result(res: i32, transferred: u32) -> Poll<io::Result<usize>> {
    if res == 0 {
        let error = GetLastError();
        match error {
            ERROR_IO_PENDING => Poll::Pending,
            0 | ERROR_IO_INCOMPLETE | ERROR_HANDLE_EOF | ERROR_PIPE_CONNECTED | ERROR_NO_DATA => {
                Poll::Ready(Ok(0))
            }
            _ => Poll::Ready(Err(io::Error::from_raw_os_error(error as _))),
        }
    } else {
        Poll::Ready(Ok(transferred as _))
    }
}

impl<T: IoBufMut> OpCode for ReadAt<T> {
    unsafe fn operate(&mut self, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        if let Some(overlapped) = optr.as_mut() {
            overlapped.Anonymous.Anonymous.Offset = (self.offset & 0xFFFFFFFF) as _;
            overlapped.Anonymous.Anonymous.OffsetHigh = (self.offset >> 32) as _;
        }
        let mut read = 0;
        let slice = self.buffer.as_buf_mut();
        let res = ReadFile(
            self.fd as _,
            slice.as_mut_ptr() as _,
            slice.len() as _,
            &mut read,
            optr,
        );
        win32_result(res, read)
    }
}

impl<T: IoBuf> OpCode for WriteAt<T> {
    unsafe fn operate(&mut self, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        if let Some(overlapped) = optr.as_mut() {
            overlapped.Anonymous.Anonymous.Offset = (self.offset & 0xFFFFFFFF) as _;
            overlapped.Anonymous.Anonymous.OffsetHigh = (self.offset >> 32) as _;
        }
        let mut written = 0;
        let slice = self.buffer.as_buf();
        let res = WriteFile(
            self.fd as _,
            slice.as_ptr() as _,
            slice.len() as _,
            &mut written,
            optr,
        );
        win32_result(res, written)
    }
}
