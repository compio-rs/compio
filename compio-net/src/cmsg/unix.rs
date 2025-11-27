use libc::{CMSG_DATA, CMSG_FIRSTHDR, CMSG_LEN, CMSG_NXTHDR, CMSG_SPACE, c_int, cmsghdr, msghdr};

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

    pub(crate) fn is_space_enough<T>(&self) -> bool {
        if !self.cmsg.is_null() {
            let space = unsafe { CMSG_SPACE(std::mem::size_of::<T>() as _) as usize };
            #[allow(clippy::unnecessary_cast)]
            let max = self.msg.msg_control as usize + self.msg.msg_controllen as usize;
            self.cmsg as usize + space <= max
        } else {
            false
        }
    }
}
