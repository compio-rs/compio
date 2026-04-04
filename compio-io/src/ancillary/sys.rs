use std::{mem::MaybeUninit, slice};

#[cfg(unix)]
use libc::{CMSG_DATA, CMSG_FIRSTHDR, CMSG_LEN, CMSG_NXTHDR, msghdr};
#[cfg(unix)]
pub use libc::{CMSG_SPACE, cmsghdr};
#[cfg(windows)]
pub use windows_sys::Win32::Networking::WinSock::CMSGHDR as cmsghdr;
#[cfg(windows)]
use windows_sys::Win32::Networking::WinSock::{self, IN_PKTINFO, IN6_PKTINFO};

use super::{AncillaryData, CodecError, copy_from_bytes, copy_to_bytes};

#[cfg(windows)]
#[allow(non_snake_case)]
mod windows_macros {
    use std::ptr::null_mut;

    use windows_sys::Win32::Networking::WinSock::{CMSGHDR, WSABUF, WSAMSG};

    const fn CMSG_ALIGN(length: usize) -> usize {
        (length + align_of::<CMSGHDR>() - 1) & !(align_of::<CMSGHDR>() - 1)
    }

    const WSA_CMSGDATA_OFFSET: usize = CMSG_ALIGN(size_of::<CMSGHDR>());

    pub unsafe fn CMSG_DATA(cmsg: *const CMSGHDR) -> *mut u8 {
        unsafe { cmsg.offset(1) as *mut u8 }
    }

    pub const unsafe fn CMSG_SPACE(length: usize) -> usize {
        WSA_CMSGDATA_OFFSET + CMSG_ALIGN(length)
    }

    pub const unsafe fn CMSG_LEN(length: usize) -> usize {
        WSA_CMSGDATA_OFFSET + length
    }

    pub unsafe fn CMSG_FIRSTHDR(msg: *const WSAMSG) -> *mut CMSGHDR {
        unsafe {
            if (*msg).Control.len as usize >= size_of::<CMSGHDR>() {
                (*msg).Control.buf as _
            } else {
                null_mut()
            }
        }
    }

    pub unsafe fn CMSG_NXTHDR(msg: *const WSAMSG, cmsg: *const CMSGHDR) -> *mut CMSGHDR {
        unsafe {
            if cmsg.is_null() {
                CMSG_FIRSTHDR(msg)
            } else {
                let next = cmsg as usize + CMSG_ALIGN((*cmsg).cmsg_len);
                if next + size_of::<CMSGHDR>()
                    > (*msg).Control.buf as usize + (*msg).Control.len as usize
                {
                    null_mut()
                } else {
                    next as _
                }
            }
        }
    }

    pub fn msghdr_from_raw(ptr: *const u8, len: usize) -> WSAMSG {
        WSAMSG {
            Control: WSABUF {
                len: len as _,
                buf: ptr as _,
            },
            ..unsafe { std::mem::zeroed() }
        }
    }
}

#[cfg(windows)]
pub use windows_macros::CMSG_SPACE;
#[cfg(windows)]
use windows_macros::{CMSG_DATA, CMSG_FIRSTHDR, CMSG_LEN, CMSG_NXTHDR, msghdr_from_raw};

#[cfg(unix)]
fn msghdr_from_raw(ptr: *const u8, len: usize) -> msghdr {
    let mut msg: msghdr = unsafe { std::mem::zeroed() };
    msg.msg_control = ptr as _;
    msg.msg_controllen = len as _;
    msg
}

pub(crate) struct CMsgRef<'a>(&'a cmsghdr);

impl CMsgRef<'_> {
    pub(crate) fn level(&self) -> i32 {
        self.0.cmsg_level as _
    }

    pub(crate) fn ty(&self) -> i32 {
        self.0.cmsg_type as _
    }

    pub(crate) fn len(&self) -> usize {
        self.0.cmsg_len as _
    }

    pub(crate) fn decode_data<T: AncillaryData>(&self) -> Result<T, CodecError> {
        let data_ptr = unsafe { CMSG_DATA(self.0) } as *const u8;
        let buffer = unsafe { slice::from_raw_parts(data_ptr, self.len()) };
        T::decode(buffer)
    }
}

pub(crate) struct CMsgMut<'a>(&'a mut cmsghdr);

impl CMsgMut<'_> {
    pub(crate) fn set_level(&mut self, level: i32) {
        self.0.cmsg_level = level as _;
    }

    pub(crate) fn set_ty(&mut self, ty: i32) {
        self.0.cmsg_type = ty as _;
    }

    pub(crate) fn encode_data<T: AncillaryData>(&mut self, value: &T) -> Result<usize, CodecError> {
        self.0.cmsg_len = unsafe { CMSG_LEN(T::SIZE as _) } as _;
        let data_ptr = unsafe { CMSG_DATA(self.0) } as *mut MaybeUninit<u8>;
        let buffer = unsafe { slice::from_raw_parts_mut(data_ptr, T::SIZE) };
        value.encode(buffer)?;
        Ok(unsafe { CMSG_SPACE(T::SIZE as _) } as _)
    }
}

pub(crate) struct CMsgIter {
    len: usize,
    offset: Option<usize>,
}

impl CMsgIter {
    pub(crate) fn new(ptr: *const u8, len: usize) -> Self {
        assert!(len >= unsafe { CMSG_SPACE(0) as _ }, "buffer too short");
        assert!(ptr.cast::<cmsghdr>().is_aligned(), "misaligned buffer");

        let msg = msghdr_from_raw(ptr.cast_mut(), len);
        let first_cmsg = unsafe { CMSG_FIRSTHDR(&msg) };

        let offset = if first_cmsg.is_null() {
            None
        } else {
            Some(first_cmsg.addr() - ptr.addr())
        };
        Self { len, offset }
    }

    pub(crate) unsafe fn current<'a>(&self, ptr: *const u8) -> Option<CMsgRef<'a>> {
        self.offset
            .and_then(|offset| unsafe { ptr.add(offset).cast::<cmsghdr>().as_ref() })
            .map(CMsgRef)
    }

    pub(crate) unsafe fn next(&mut self, ptr: *const u8) {
        if let Some(offset) = self.offset {
            let msg = msghdr_from_raw(ptr, self.len);
            let next_cmsg = unsafe { CMSG_NXTHDR(&msg, ptr.add(offset).cast()) };
            if next_cmsg.is_null() {
                self.offset = None;
            } else {
                self.offset = Some(next_cmsg.addr() - ptr.addr());
            }
        }
    }

    pub(crate) unsafe fn current_mut<'a>(&self, ptr: *mut u8) -> Option<CMsgMut<'a>> {
        self.offset
            .and_then(|offset| unsafe { ptr.add(offset).cast::<cmsghdr>().as_mut() })
            .map(CMsgMut)
    }

    pub(crate) fn is_space_enough(&self, space: usize) -> bool {
        if let Some(offset) = self.offset {
            #[allow(clippy::unnecessary_cast)]
            let space = unsafe { CMSG_SPACE(space as _) } as usize;
            offset + space <= self.len
        } else {
            false
        }
    }
}

#[cfg(unix)]
impl AncillaryData for libc::in_addr {
    fn encode(&self, buffer: &mut [MaybeUninit<u8>]) -> Result<(), CodecError> {
        unsafe { copy_to_bytes(self, buffer) }
    }

    fn decode(buffer: &[u8]) -> Result<Self, CodecError> {
        unsafe { copy_from_bytes(buffer) }
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
impl AncillaryData for libc::in_pktinfo {
    fn encode(&self, buffer: &mut [MaybeUninit<u8>]) -> Result<(), CodecError> {
        let mut pktinfo: libc::in_pktinfo = unsafe { std::mem::zeroed() };
        pktinfo.ipi_ifindex = self.ipi_ifindex;
        pktinfo.ipi_spec_dst.s_addr = self.ipi_spec_dst.s_addr;
        pktinfo.ipi_addr.s_addr = self.ipi_addr.s_addr;
        unsafe { copy_to_bytes(&pktinfo, buffer) }
    }

    fn decode(buffer: &[u8]) -> Result<Self, CodecError> {
        let pktinfo: libc::in_pktinfo = unsafe { copy_from_bytes(buffer) }?;
        Ok(libc::in_pktinfo {
            ipi_ifindex: pktinfo.ipi_ifindex,
            ipi_spec_dst: libc::in_addr {
                s_addr: pktinfo.ipi_spec_dst.s_addr,
            },
            ipi_addr: libc::in_addr {
                s_addr: pktinfo.ipi_addr.s_addr,
            },
        })
    }
}

#[cfg(unix)]
impl AncillaryData for libc::in6_pktinfo {
    fn encode(&self, buffer: &mut [MaybeUninit<u8>]) -> Result<(), CodecError> {
        let mut pktinfo: libc::in6_pktinfo = unsafe { std::mem::zeroed() };
        pktinfo.ipi6_ifindex = self.ipi6_ifindex;
        pktinfo.ipi6_addr.s6_addr = self.ipi6_addr.s6_addr;
        unsafe { copy_to_bytes(&pktinfo, buffer) }
    }

    fn decode(buffer: &[u8]) -> Result<Self, CodecError> {
        let pktinfo: libc::in6_pktinfo = unsafe { copy_from_bytes(buffer) }?;
        Ok(libc::in6_pktinfo {
            ipi6_ifindex: pktinfo.ipi6_ifindex,
            ipi6_addr: libc::in6_addr {
                s6_addr: pktinfo.ipi6_addr.s6_addr,
            },
        })
    }
}

#[cfg(windows)]
impl AncillaryData for IN_PKTINFO {
    fn encode(&self, buffer: &mut [MaybeUninit<u8>]) -> Result<(), CodecError> {
        let mut pktinfo: IN_PKTINFO = unsafe { std::mem::zeroed() };
        unsafe {
            pktinfo.ipi_addr.S_un.S_addr = self.ipi_addr.S_un.S_addr;
        }
        pktinfo.ipi_ifindex = self.ipi_ifindex;
        unsafe { copy_to_bytes(&pktinfo, buffer) }
    }

    fn decode(buffer: &[u8]) -> Result<Self, CodecError> {
        let pktinfo: IN_PKTINFO = unsafe { copy_from_bytes(buffer) }?;
        Ok(IN_PKTINFO {
            ipi_addr: WinSock::IN_ADDR {
                S_un: WinSock::IN_ADDR_0 {
                    S_addr: unsafe { pktinfo.ipi_addr.S_un.S_addr },
                },
            },
            ipi_ifindex: pktinfo.ipi_ifindex,
        })
    }
}

#[cfg(windows)]
impl AncillaryData for IN6_PKTINFO {
    fn encode(&self, buffer: &mut [MaybeUninit<u8>]) -> Result<(), CodecError> {
        let mut pktinfo: IN6_PKTINFO = unsafe { std::mem::zeroed() };
        unsafe {
            pktinfo.ipi6_addr.u.Byte = self.ipi6_addr.u.Byte;
        }
        pktinfo.ipi6_ifindex = self.ipi6_ifindex;
        unsafe { copy_to_bytes(&pktinfo, buffer) }
    }

    fn decode(buffer: &[u8]) -> Result<Self, CodecError> {
        let pktinfo: IN6_PKTINFO = unsafe { copy_from_bytes(buffer) }?;
        Ok(IN6_PKTINFO {
            ipi6_addr: WinSock::IN6_ADDR {
                u: WinSock::IN6_ADDR_0 {
                    Byte: unsafe { pktinfo.ipi6_addr.u.Byte },
                },
            },
            ipi6_ifindex: pktinfo.ipi6_ifindex,
        })
    }
}
