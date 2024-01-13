use std::{
    io,
    net::SocketAddr,
    ptr::{null, null_mut},
    task::Poll,
};

use compio_driver::syscall;
use futures_channel::oneshot::{channel, Sender};
use socket2::SockAddr;
use widestring::U16CString;
use windows_sys::Win32::{
    Foundation::{GetLastError, ERROR_IO_PENDING, HANDLE},
    Networking::WinSock::{
        FreeAddrInfoExW, GetAddrInfoExCancel, GetAddrInfoExOverlappedResult, GetAddrInfoExW,
        ADDRINFOEXW, AF_UNSPEC, IPPROTO_TCP, NS_ALL, SOCK_STREAM,
    },
    System::IO::OVERLAPPED,
};

struct AsyncResolver {
    name: U16CString,
    port: u16,
    result: *mut ADDRINFOEXW,
    handle: HANDLE,
    overlapped: GAIOverlapped,
}

impl AsyncResolver {
    pub fn new(name: &str, port: u16) -> io::Result<Self> {
        Ok(Self {
            name: U16CString::from_str(name)
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid host name"))?,
            port,
            result: null_mut(),
            handle: 0,
            overlapped: GAIOverlapped::new(),
        })
    }

    unsafe extern "system" fn callback(
        _dwerror: u32,
        _dwbytes: u32,
        lpoverlapped: *const OVERLAPPED,
    ) {
        // We won't access the overlapped struct outside callback.
        let overlapped_ptr = lpoverlapped.cast::<GAIOverlapped>().cast_mut();
        if let Some(overlapped) = overlapped_ptr.as_mut() {
            if let Some(handle) = overlapped.handle.take() {
                handle.send(()).ok();
            }
        }
    }

    pub unsafe fn call(&mut self, hints: &ADDRINFOEXW, handle: Sender<()>) -> Poll<io::Result<()>> {
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
            &mut self.handle,
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
        self.handle = 0;

        let mut addrs = vec![];
        let mut result = self.result;
        while let Some(info) = unsafe { result.as_ref() } {
            let addr = unsafe {
                SockAddr::try_init(|buffer, len| {
                    std::slice::from_raw_parts_mut::<u8>(buffer.cast(), info.ai_addrlen as _)
                        .copy_from_slice(std::slice::from_raw_parts::<u8>(
                            info.ai_addr.cast(),
                            info.ai_addrlen as _,
                        ));
                    *len = info.ai_addrlen as _;
                    Ok(())
                })
            }
            // it is always Ok
            .unwrap()
            .1;
            if let Some(mut addr) = addr.as_socket() {
                addr.set_port(self.port);
                addrs.push(addr)
            }
            result = info.ai_next;
        }
        Ok(addrs.into_iter())
    }
}

impl Drop for AsyncResolver {
    fn drop(&mut self) {
        if self.handle != 0 {
            syscall!(SOCKET, GetAddrInfoExCancel(&self.handle)).ok();
        }
        if !self.result.is_null() {
            unsafe { FreeAddrInfoExW(self.result) }
        }
    }
}

#[repr(C)]
struct GAIOverlapped {
    base: OVERLAPPED,
    handle: Option<Sender<()>>,
}

impl GAIOverlapped {
    pub fn new() -> Self {
        Self {
            base: unsafe { std::mem::zeroed() },
            handle: None,
        }
    }
}

pub async fn resolve_sock_addrs(
    host: &str,
    port: u16,
) -> io::Result<std::vec::IntoIter<SocketAddr>> {
    let mut resolver = AsyncResolver::new(host, port)?;
    let mut hints: ADDRINFOEXW = unsafe { std::mem::zeroed() };
    hints.ai_family = AF_UNSPEC as _;
    hints.ai_socktype = SOCK_STREAM;
    hints.ai_protocol = IPPROTO_TCP;

    let (sender, receiver) = channel::<()>();
    match unsafe { resolver.call(&hints, sender) } {
        Poll::Ready(res) => {
            res?;
        }
        Poll::Pending => {
            receiver
                .await
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        }
    }

    unsafe { resolver.addrs() }
}
