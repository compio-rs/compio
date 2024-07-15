use std::{
    mem::{align_of, size_of},
    ptr::null_mut,
};

use windows_sys::Win32::Networking::WinSock::{CMSGHDR, WSABUF, WSAMSG};

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
    if (*msg).Control.len as usize >= size_of::<CMSGHDR>() {
        (*msg).Control.buf as _
    } else {
        null_mut()
    }
}

#[inline]
unsafe fn wsa_cmsg_nxthdr(msg: *const WSAMSG, cmsg: *const CMSGHDR) -> *mut CMSGHDR {
    if cmsg.is_null() {
        wsa_cmsg_firsthdr(msg)
    } else {
        let next = cmsg as usize + wsa_cmsghdr_align((*cmsg).cmsg_len);
        if next + size_of::<CMSGHDR>() > (*msg).Control.buf as usize + (*msg).Control.len as usize {
            null_mut()
        } else {
            next as _
        }
    }
}

#[inline]
unsafe fn wsa_cmsg_data(cmsg: *const CMSGHDR) -> *mut u8 {
    (cmsg as usize + WSA_CMSGDATA_OFFSET) as _
}

#[inline]
const fn wsa_cmsg_space(length: usize) -> usize {
    WSA_CMSGDATA_OFFSET + wsa_cmsghdr_align(length)
}

#[inline]
const fn wsa_cmsg_len(length: usize) -> usize {
    WSA_CMSGDATA_OFFSET + length
}

pub struct CMsgRef<'a>(&'a CMSGHDR);

impl<'a> CMsgRef<'a> {
    pub fn level(&self) -> i32 {
        self.0.cmsg_level
    }

    pub fn ty(&self) -> i32 {
        self.0.cmsg_type
    }

    pub fn len(&self) -> usize {
        self.0.cmsg_len
    }

    pub unsafe fn data<T>(&self) -> &T {
        let data_ptr = wsa_cmsg_data(self.0);
        data_ptr.cast::<T>().as_ref().unwrap()
    }
}

pub(crate) struct CMsgMut<'a>(&'a mut CMSGHDR);

impl<'a> CMsgMut<'a> {
    pub(crate) fn set_level(&mut self, level: i32) {
        self.0.cmsg_level = level;
    }

    pub(crate) fn set_ty(&mut self, ty: i32) {
        self.0.cmsg_type = ty;
    }

    pub(crate) unsafe fn set_data<T>(&mut self, data: T) -> usize {
        self.0.cmsg_len = wsa_cmsg_len(size_of::<T>() as _) as _;
        let data_ptr = wsa_cmsg_data(self.0);
        std::ptr::write(data_ptr.cast::<T>(), data);
        wsa_cmsg_space(size_of::<T>() as _)
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
        self.cmsg.as_ref().map(CMsgRef)
    }

    pub(crate) unsafe fn next(&mut self) {
        if !self.cmsg.is_null() {
            self.cmsg = wsa_cmsg_nxthdr(&self.msg, self.cmsg);
        }
    }

    pub(crate) unsafe fn current_mut<'a>(&self) -> Option<CMsgMut<'a>> {
        self.cmsg.as_mut().map(CMsgMut)
    }

    pub(crate) fn is_aligned<T>(&self) -> bool {
        self.msg.Control.buf.cast::<T>().is_aligned()
    }

    pub(crate) fn is_space_enough<T>(&self) -> bool {
        if !self.cmsg.is_null() {
            let space = wsa_cmsg_space(size_of::<T>() as _);
            let max = self.msg.Control.buf as usize + self.msg.Control.len as usize;
            self.cmsg as usize + space <= max
        } else {
            false
        }
    }
}
