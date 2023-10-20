use std::{
    io,
    net::SocketAddr,
    ptr::{null, null_mut},
    task::Poll,
};

use compio_driver::syscall;
use compio_runtime::event::EventHandle;
use widestring::U16CString;
pub use windows_sys::Win32::Networking::WinSock::{
    ADDRINFOEXW as addrinfo, AF_UNSPEC, IPPROTO_TCP, SOCKADDR_STORAGE as sockaddr_storage,
    SOCK_STREAM,
};
use windows_sys::Win32::{
    Foundation::{GetLastError, ERROR_IO_PENDING},
    Networking::WinSock::{
        FreeAddrInfoExW, GetAddrInfoExOverlappedResult, GetAddrInfoExW, ADDRINFOEXW, NS_ALL,
    },
    System::IO::OVERLAPPED,
};

pub struct AsyncResolver {
    name: U16CString,
    port: u16,
    result: *mut ADDRINFOEXW,
    overlapped: GAIOverlapped,
}

impl AsyncResolver {
    pub fn new(name: &str, port: u16) -> io::Result<Self> {
        Ok(Self {
            name: U16CString::from_str(name)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid host name"))?,
            port,
            result: null_mut(),
            overlapped: GAIOverlapped::new(),
        })
    }

    unsafe extern "system" fn callback(
        _dwerror: u32,
        _dwbytes: u32,
        lpoverlapped: *const OVERLAPPED,
    ) {
        let overlapped_ptr = lpoverlapped.cast::<GAIOverlapped>();
        if let Some(overlapped) = overlapped_ptr.as_ref() {
            if let Some(handle) = overlapped.handle.as_ref() {
                handle.notify().ok();
            }
        }
    }

    pub unsafe fn call(
        &mut self,
        hints: &ADDRINFOEXW,
        handle: EventHandle,
    ) -> Poll<io::Result<()>> {
        self.overlapped.handle = Some(handle);
        let res = GetAddrInfoExW(
            self.name.as_ptr(),
            null(),
            NS_ALL,
            null(),
            hints,
            &mut self.result,
            null(),
            &self.overlapped.base,
            Some(Self::callback),
            null_mut(),
        );
        match res {
            0 => Poll::Ready(Ok(())),
            _ => {
                let code = GetLastError();
                match code {
                    ERROR_IO_PENDING => Poll::Pending,
                    _ => Poll::Ready(Err(io::Error::from_raw_os_error(code as _))),
                }
            }
        }
    }

    pub unsafe fn addrs(&mut self) -> io::Result<std::vec::IntoIter<SocketAddr>> {
        syscall!(SOCKET, GetAddrInfoExOverlappedResult(&self.overlapped.base))?;
        Ok(super::to_addrs(self.result, self.port))
    }
}

impl Drop for AsyncResolver {
    fn drop(&mut self) {
        if !self.result.is_null() {
            unsafe { FreeAddrInfoExW(self.result) }
        }
    }
}

#[repr(C)]
struct GAIOverlapped {
    base: OVERLAPPED,
    handle: Option<EventHandle>,
}

impl GAIOverlapped {
    pub fn new() -> Self {
        Self {
            base: unsafe { std::mem::zeroed() },
            handle: None,
        }
    }
}
