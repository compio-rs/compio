use std::{mem::MaybeUninit, slice};

use libc::{CMSG_DATA, CMSG_FIRSTHDR, CMSG_LEN, CMSG_NXTHDR, CMSG_SPACE, c_int, cmsghdr, msghdr};

use super::{AncillaryData, CodecError, copy_from_bytes, copy_to_bytes};

pub(crate) struct CMsgRef<'a>(&'a cmsghdr);

impl CMsgRef<'_> {
    pub(crate) fn level(&self) -> c_int {
        self.0.cmsg_level
    }

    pub(crate) fn ty(&self) -> c_int {
        self.0.cmsg_type
    }

    pub(crate) fn len(&self) -> usize {
        self.0.cmsg_len as _
    }

    pub(crate) fn decode_data<T: AncillaryData>(&self) -> Result<T, CodecError> {
        let data_ptr = unsafe { CMSG_DATA(self.0) } as *const u8;
        let buffer = unsafe { slice::from_raw_parts(data_ptr, self.len()) };
        T::decode(buffer)
    }

    pub(crate) unsafe fn data<T>(&self) -> &T {
        unsafe {
            let data_ptr = CMSG_DATA(self.0);
            data_ptr.cast::<T>().as_ref().unwrap()
        }
    }
}

pub(crate) struct CMsgMut<'a>(&'a mut cmsghdr);

impl CMsgMut<'_> {
    pub(crate) fn set_level(&mut self, level: c_int) {
        self.0.cmsg_level = level;
    }

    pub(crate) fn set_ty(&mut self, ty: c_int) {
        self.0.cmsg_type = ty;
    }

    pub(crate) fn encode_data<T: AncillaryData>(&mut self, value: &T) -> Result<usize, CodecError> {
        self.0.cmsg_len = unsafe { CMSG_LEN(T::SIZE as _) } as _;
        let data_ptr = unsafe { CMSG_DATA(self.0) } as *mut MaybeUninit<u8>;
        let buffer = unsafe { slice::from_raw_parts_mut(data_ptr, T::SIZE) };
        value.encode(buffer)?;
        Ok(unsafe { CMSG_SPACE(T::SIZE as _) } as _)
    }

    pub(crate) unsafe fn set_data<T>(&mut self, data: T) -> usize {
        unsafe {
            self.0.cmsg_len = CMSG_LEN(std::mem::size_of::<T>() as _) as _;
            let data_ptr = CMSG_DATA(self.0);
            std::ptr::write(data_ptr.cast::<T>(), data);
            CMSG_SPACE(std::mem::size_of::<T>() as _) as _
        }
    }
}

pub(crate) struct CMsgIter {
    msg: msghdr,
    cmsg: *mut cmsghdr,
}

impl CMsgIter {
    pub(crate) fn new(ptr: *const u8, len: usize) -> Self {
        assert!(len >= unsafe { CMSG_SPACE(0) as _ }, "buffer too short");
        assert!(ptr.cast::<cmsghdr>().is_aligned(), "misaligned buffer");

        let mut msg: msghdr = unsafe { std::mem::zeroed() };
        msg.msg_control = ptr as _;
        msg.msg_controllen = len as _;
        // SAFETY: msg is initialized and valid
        let cmsg = unsafe { CMSG_FIRSTHDR(&msg) };
        Self { msg, cmsg }
    }

    pub(crate) unsafe fn current<'a>(&self) -> Option<CMsgRef<'a>> {
        // SAFETY: cmsg is valid or null
        unsafe { self.cmsg.as_ref() }.map(CMsgRef)
    }

    pub(crate) unsafe fn next(&mut self) {
        if !self.cmsg.is_null() {
            // SAFETY: msg and cmsg are valid
            self.cmsg = unsafe { CMSG_NXTHDR(&self.msg, self.cmsg) };
        }
    }

    pub(crate) unsafe fn current_mut<'a>(&self) -> Option<CMsgMut<'a>> {
        // SAFETY: cmsg is valid or null
        unsafe { self.cmsg.as_mut() }.map(CMsgMut)
    }

    pub(crate) fn is_aligned<T>(&self) -> bool {
        self.msg.msg_control.cast::<T>().is_aligned()
    }

    pub(crate) fn is_space_enough(&self, space: usize) -> bool {
        if !self.cmsg.is_null() {
            let space = unsafe { CMSG_SPACE(space as _) as usize };
            #[allow(clippy::unnecessary_cast)]
            let max = self.msg.msg_control as usize + self.msg.msg_controllen as usize;
            self.cmsg as usize + space <= max
        } else {
            false
        }
    }
}

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
