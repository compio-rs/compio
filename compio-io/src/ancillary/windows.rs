use std::{
    mem::{MaybeUninit, align_of, size_of},
    ptr::null_mut,
    slice,
};

use windows_sys::Win32::Networking::WinSock::{
    self, CMSGHDR, IN_PKTINFO, IN6_PKTINFO, WSABUF, WSAMSG,
};

use super::{AncillaryData, CodecError, copy_from_bytes, copy_to_bytes};

// Macros from https://github.com/microsoft/win32metadata/blob/main/generation/WinSDK/RecompiledIdlHeaders/shared/ws2def.h
#[inline]
const fn wsa_cmsghdr_align(length: usize) -> usize {
    (length + align_of::<CMSGHDR>() - 1) & !(align_of::<CMSGHDR>() - 1)
}

// WSA_CMSGDATA_ALIGN(sizeof(CMSGHDR))
const WSA_CMSGDATA_OFFSET: usize =
    (size_of::<CMSGHDR>() + align_of::<usize>() - 1) & !(align_of::<usize>() - 1);

#[inline]
unsafe fn wsa_cmsg_firsthdr(msg: *const WSAMSG) -> *mut CMSGHDR {
    unsafe {
        if (*msg).Control.len as usize >= size_of::<CMSGHDR>() {
            (*msg).Control.buf as _
        } else {
            null_mut()
        }
    }
}

#[inline]
unsafe fn wsa_cmsg_nxthdr(msg: *const WSAMSG, cmsg: *const CMSGHDR) -> *mut CMSGHDR {
    unsafe {
        if cmsg.is_null() {
            wsa_cmsg_firsthdr(msg)
        } else {
            let next = cmsg as usize + wsa_cmsghdr_align((*cmsg).cmsg_len);
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

#[inline]
unsafe fn wsa_cmsg_data(cmsg: *const CMSGHDR) -> *mut u8 {
    (cmsg as usize + WSA_CMSGDATA_OFFSET) as _
}

#[inline]
pub(crate) const fn wsa_cmsg_space(length: usize) -> usize {
    WSA_CMSGDATA_OFFSET + wsa_cmsghdr_align(length)
}

#[inline]
const fn wsa_cmsg_len(length: usize) -> usize {
    WSA_CMSGDATA_OFFSET + length
}

pub struct CMsgRef<'a>(&'a CMSGHDR);

impl CMsgRef<'_> {
    pub fn level(&self) -> i32 {
        self.0.cmsg_level
    }

    pub fn ty(&self) -> i32 {
        self.0.cmsg_type
    }

    pub fn len(&self) -> usize {
        self.0.cmsg_len
    }

    pub fn decode_data<T: AncillaryData>(&self) -> Result<T, CodecError> {
        let data_ptr = unsafe { wsa_cmsg_data(self.0) } as *const u8;
        let buffer = unsafe { slice::from_raw_parts(data_ptr, self.len()) };
        T::decode(buffer)
    }
}

pub(crate) struct CMsgMut<'a>(&'a mut CMSGHDR);

impl CMsgMut<'_> {
    pub(crate) fn set_level(&mut self, level: i32) {
        self.0.cmsg_level = level;
    }

    pub(crate) fn set_ty(&mut self, ty: i32) {
        self.0.cmsg_type = ty;
    }

    pub(crate) fn encode_data<T: AncillaryData>(&mut self, value: &T) -> Result<usize, CodecError> {
        let data_ptr = unsafe { wsa_cmsg_data(self.0) } as *mut MaybeUninit<u8>;
        let buffer = unsafe { slice::from_raw_parts_mut(data_ptr, T::SIZE) };
        value.encode(buffer)?;
        self.0.cmsg_len = wsa_cmsg_len(T::SIZE as _) as _;
        Ok(wsa_cmsg_space(T::SIZE as _))
    }
}

pub(crate) struct CMsgIter {
    msg: WSAMSG,
    cmsg: *mut CMSGHDR,
}

impl CMsgIter {
    pub(crate) fn new(ptr: *const u8, len: usize) -> Self {
        assert!(len >= wsa_cmsg_space(0) as _, "buffer too short");
        assert!(ptr.cast::<CMSGHDR>().is_aligned(), "misaligned buffer");

        let mut msg: WSAMSG = unsafe { std::mem::zeroed() };
        msg.Control = WSABUF {
            len: len as _,
            buf: ptr as _,
        };
        // SAFETY: msg is initialized and valid
        let cmsg = unsafe { wsa_cmsg_firsthdr(&msg) };
        Self { msg, cmsg }
    }

    pub(crate) unsafe fn current<'a>(&self) -> Option<CMsgRef<'a>> {
        // SAFETY: cmsg is valid or null
        unsafe { self.cmsg.as_ref() }.map(CMsgRef)
    }

    pub(crate) unsafe fn next(&mut self) {
        if !self.cmsg.is_null() {
            // SAFETY: msg and cmsg are valid
            self.cmsg = unsafe { wsa_cmsg_nxthdr(&self.msg, self.cmsg) };
        }
    }

    pub(crate) unsafe fn current_mut<'a>(&self) -> Option<CMsgMut<'a>> {
        // SAFETY: cmsg is valid or null
        unsafe { self.cmsg.as_mut() }.map(CMsgMut)
    }

    pub(crate) fn is_space_enough(&self, space: usize) -> bool {
        if !self.cmsg.is_null() {
            let space = wsa_cmsg_space(space as _);
            let max = self.msg.Control.buf as usize + self.msg.Control.len as usize;
            self.cmsg as usize + space <= max
        } else {
            false
        }
    }
}

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
