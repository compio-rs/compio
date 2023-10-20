use std::{ffi::CString, io, net::SocketAddr, ops::DerefMut, pin::Pin, task::Poll};

use compio_driver::{FromRawFd, IntoRawFd, RawFd};
use compio_runtime::event::EventHandle;
pub use libc::{addrinfo, sockaddr_storage, AF_UNSPEC, IPPROTO_TCP, SOCK_STREAM};

#[repr(C)]
#[allow(non_camel_case_types)]
struct gaicb {
    ar_name: *const libc::c_char,
    ar_service: *const libc::c_char,
    ar_request: *const libc::addrinfo,
    ar_result: *mut libc::addrinfo,
}

#[repr(C)]
#[allow(non_camel_case_types)]
struct sigevent_thread {
    pub sigev_value: libc::sigval,
    pub sigev_signo: libc::c_int,
    pub sigev_notify: libc::c_int,
    pub sigev_notify_function: Option<unsafe extern "C" fn(libc::sigval)>,
    pub sigev_notify_attributes: *mut libc::pthread_attr_t,
    #[cfg(target_pointer_width = "64")]
    __unused1: [libc::c_int; 8],
    #[cfg(target_pointer_width = "32")]
    __unused1: [libc::c_int; 11],
}

#[link(name = "anl")]
#[allow(unused)]
extern "C" {
    fn getaddrinfo_a(
        mode: libc::c_int,
        list: *mut *mut gaicb,
        nitems: libc::c_int,
        sevp: *mut sigevent_thread,
    ) -> libc::c_int;

    fn gai_suspend(
        list: *const *const gaicb,
        nitems: libc::c_int,
        timeout: *const libc::timespec,
    ) -> libc::c_int;

    fn gai_error(req: *mut gaicb) -> libc::c_int;

    fn gai_cancel(req: *mut gaicb) -> libc::c_int;
}

const GAI_NOWAIT: libc::c_int = 1;

const EAI_INPROGRESS: libc::c_int = -100;
const EAI_INTR: libc::c_int = -104;

fn gai_call(res: libc::c_int) -> io::Result<()> {
    let errno = match res {
        0 => return Ok(()),
        libc::EAI_AGAIN => libc::EAGAIN,
        libc::EAI_MEMORY => libc::ENOMEM,
        libc::EAI_SYSTEM => libc::EINVAL,
        EAI_INPROGRESS => libc::EINPROGRESS,
        EAI_INTR => libc::EINTR,
        _ => {
            let detail = unsafe {
                std::str::from_utf8(std::ffi::CStr::from_ptr(libc::gai_strerror(res)).to_bytes())
                    .unwrap()
            };
            return Err(io::Error::new(
                io::ErrorKind::Other,
                &format!("failed to lookup address information: {detail}")[..],
            ));
        }
    };
    Err(io::Error::from_raw_os_error(errno))
}

struct GaiControlBlock {
    block: gaicb,
}

impl GaiControlBlock {
    pub fn new() -> Self {
        Self {
            block: unsafe { std::mem::zeroed() },
        }
    }

    pub fn as_mut_ptr(&mut self) -> *mut gaicb {
        &mut self.block
    }
}

impl Drop for GaiControlBlock {
    fn drop(&mut self) {
        if !self.block.ar_result.is_null() {
            unsafe { libc::freeaddrinfo(self.block.ar_result) }
        }
    }
}

pub struct AsyncResolver {
    name: CString,
    port: u16,
    block: Pin<Box<GaiControlBlock>>,
}

impl AsyncResolver {
    pub fn new(name: &str, port: u16) -> io::Result<Self> {
        let name = CString::new(name)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid host"))?;
        Ok(Self {
            name,
            port,
            block: Box::pin(GaiControlBlock::new()),
        })
    }

    unsafe extern "C" fn callback(v: libc::sigval) {
        let handle = EventHandle::from_raw_fd(v.sival_ptr as RawFd);
        handle.notify().ok();
    }

    pub unsafe fn call(
        &mut self,
        hints: &libc::addrinfo,
        handle: EventHandle,
    ) -> Poll<io::Result<()>> {
        self.block.block.ar_name = self.name.as_ptr();
        self.block.block.ar_request = hints;

        let mut block_ptr = self.block.deref_mut().as_mut_ptr();
        let mut sevp: sigevent_thread = std::mem::zeroed();
        sevp.sigev_value.sival_ptr = handle.into_raw_fd() as _;
        sevp.sigev_notify = libc::SIGEV_THREAD;
        sevp.sigev_notify_function = Some(Self::callback);

        gai_call(getaddrinfo_a(GAI_NOWAIT, &mut block_ptr, 1, &mut sevp))?;
        Poll::Pending
    }

    pub unsafe fn addrs(&mut self) -> io::Result<std::vec::IntoIter<SocketAddr>> {
        gai_call(gai_error(self.block.deref_mut().as_mut_ptr()))?;

        Ok(super::to_addrs(self.block.block.ar_result, self.port))
    }
}
