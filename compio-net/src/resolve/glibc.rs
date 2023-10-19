use std::{
    ffi::{CStr, CString},
    io,
    marker::PhantomData,
    mem::MaybeUninit,
    net::SocketAddr,
    ops::DerefMut,
    ptr::{null, null_mut},
};

use compio_driver::{FromRawFd, IntoRawFd};
use compio_runtime::event::{Event, EventHandle};
use socket2::SockAddr;

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

struct GaiControlBlock<'a> {
    block: gaicb,
    _p: PhantomData<(&'a CStr, &'a libc::addrinfo)>,
}

impl<'a> GaiControlBlock<'a> {
    pub fn new(c_host: &'a CStr, hints: &'a libc::addrinfo) -> Self {
        let block = gaicb {
            ar_name: c_host.as_ptr(),
            ar_service: null(),
            ar_request: hints,
            ar_result: null_mut(),
        };
        Self {
            block,
            _p: PhantomData,
        }
    }

    pub fn as_mut_ptr(&mut self) -> *mut gaicb {
        &mut self.block
    }

    pub fn to_addrs(&self, port: u16) -> Vec<SocketAddr> {
        let mut addrs = vec![];
        let mut result = self.block.ar_result;
        while let Some(info) = unsafe { result.as_ref() } {
            unsafe {
                let mut buffer = MaybeUninit::<libc::sockaddr_storage>::zeroed();
                std::slice::from_raw_parts_mut::<u8>(
                    buffer.as_mut_ptr().cast(),
                    info.ai_addrlen as _,
                )
                .copy_from_slice(std::slice::from_raw_parts::<u8>(
                    info.ai_addr.cast(),
                    info.ai_addrlen as _,
                ));
                let buffer = buffer.assume_init();
                let addr = SockAddr::new(buffer, info.ai_addrlen as _);
                if let Some(mut addr) = addr.as_socket() {
                    addr.set_port(port);
                    addrs.push(addr)
                }
            }
            result = info.ai_next;
        }
        addrs
    }
}

impl Drop for GaiControlBlock<'_> {
    fn drop(&mut self) {
        if !self.block.ar_result.is_null() {
            unsafe { libc::freeaddrinfo(self.block.ar_result) }
        }
    }
}

unsafe extern "C" fn callback(v: libc::sigval) {
    let handle = EventHandle::from_raw_fd(v.sival_ptr as _);
    handle.notify().ok();
}

pub async fn resolve(host: &str, port: u16) -> io::Result<Vec<SocketAddr>> {
    let c_host = CString::new(host)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid host"))?;
    let mut hints: libc::addrinfo = unsafe { std::mem::zeroed() };
    hints.ai_family = libc::AF_UNSPEC;
    hints.ai_socktype = libc::SOCK_STREAM;

    let mut block = Box::pin(GaiControlBlock::new(&c_host, &hints));
    let mut block_ptr: *mut gaicb = block.deref_mut().as_mut_ptr();

    let event = Event::new()?;
    let handle = event.handle()?;
    let handle = handle.into_raw_fd();

    let mut sevp: sigevent_thread = unsafe { std::mem::zeroed() };
    sevp.sigev_value.sival_ptr = handle as _;
    sevp.sigev_notify = libc::SIGEV_THREAD;
    sevp.sigev_notify_function = Some(callback);

    gai_call(unsafe { getaddrinfo_a(GAI_NOWAIT, &mut block_ptr, 1, &mut sevp) })?;

    event.wait().await?;

    gai_call(unsafe { gai_error(block_ptr) })?;

    Ok(block.to_addrs(port))
}
