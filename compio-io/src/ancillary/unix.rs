use std::{mem::MaybeUninit, slice};

use libc::{CMSG_DATA, CMSG_LEN, CMSG_SPACE, c_int, cmsghdr};

use super::{AncillaryData, CodecError, copy_from_bytes, copy_to_bytes};

#[inline]
const fn CMSG_ALIGN(length: usize) -> usize {
    (length + align_of::<cmsghdr>() - 1) & !(align_of::<cmsghdr>() - 1)
}

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
}

pub(crate) struct CMsgIter {
    len: usize,
    offset: Option<usize>,
}

impl CMsgIter {
    pub(crate) fn new(ptr: *const u8, len: usize) -> Self {
        assert!(len >= unsafe { CMSG_SPACE(0) as _ }, "buffer too short");
        assert!(ptr.cast::<cmsghdr>().is_aligned(), "misaligned buffer");

        let offset = if len >= size_of::<cmsghdr>() {
            Some(0)
        } else {
            None
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
            let cmsg = unsafe { ptr.add(offset).cast::<cmsghdr>().as_ref() };
            if let Some(cmsg) = cmsg {
                let offset = offset + CMSG_ALIGN(cmsg.cmsg_len);
                if offset + size_of::<cmsghdr>() <= self.len {
                    self.offset = Some(offset);
                } else {
                    self.offset = None;
                }
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
            let space = unsafe { CMSG_SPACE(space as _) } as usize;
            offset + space <= self.len
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
