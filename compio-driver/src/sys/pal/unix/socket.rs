use rustix::net::{SocketAddrAny, addr::*};

use crate::sys::prelude::*;

pub struct SockAddrArg<'a>(pub &'a SockAddr);

unsafe impl SocketAddrArg for SockAddrArg<'_> {
    unsafe fn with_sockaddr<R>(
        &self,
        f: impl FnOnce(*const SocketAddrOpaque, SocketAddrLen) -> R,
    ) -> R {
        f(self.0.as_ptr().cast(), self.0.len())
    }
}

pub fn copy_addr_from(
    storage: &mut SockAddrStorage,
    addr_len: &mut socklen_t,
    addr: Option<SocketAddrAny>,
) {
    let Some(addr) = addr else { return };

    *addr_len = addr.addr_len() as socklen_t;
    debug_assert!(*addr_len <= storage.size_of());

    unsafe {
        std::ptr::copy_nonoverlapping::<u8>(
            addr.as_ptr().cast(),
            storage as *mut _ as *mut u8,
            addr.addr_len() as usize,
        )
    };
}
