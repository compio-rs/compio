use libc::{c_int, cmsghdr, msghdr, CMSG_DATA, CMSG_FIRSTHDR, CMSG_LEN, CMSG_NXTHDR, CMSG_SPACE};

/// Reference to a control message.
pub struct CMsgRef<'a>(&'a cmsghdr);

impl<'a> CMsgRef<'a> {
    /// Returns the level of the control message.
    pub fn level(&self) -> c_int {
        self.0.cmsg_level
    }

    /// Returns the type of the control message.
    pub fn ty(&self) -> c_int {
        self.0.cmsg_type
    }

    /// Returns the length of the control message.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.0.cmsg_len as _
    }

    /// Returns a reference to the data of the control message.
    ///
    /// # Safety
    ///
    /// The data part must be properly aligned and contains an initialized
    /// instance of `T`.
    pub unsafe fn data<T>(&self) -> &T {
        let data_ptr = CMSG_DATA(self.0);
        data_ptr.cast::<T>().as_ref().unwrap()
    }
}

pub(crate) struct CMsgMut<'a>(&'a mut cmsghdr);

impl<'a> CMsgMut<'a> {
    pub(crate) fn set_level(&mut self, level: c_int) {
        self.0.cmsg_level = level;
    }

    pub(crate) fn set_ty(&mut self, ty: c_int) {
        self.0.cmsg_type = ty;
    }

    pub(crate) unsafe fn set_data<T>(&mut self, data: T) {
        self.0.cmsg_len = CMSG_LEN(std::mem::size_of::<T>() as _) as _;
        let data_ptr = CMSG_DATA(self.0);
        std::ptr::write(data_ptr.cast::<T>(), data);
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
        self.cmsg.as_ref().map(CMsgRef)
    }

    pub(crate) unsafe fn next(&mut self) {
        if !self.cmsg.is_null() {
            self.cmsg = CMSG_NXTHDR(&self.msg, self.cmsg);
        }
    }

    pub(crate) unsafe fn current_mut<'a>(&self) -> Option<CMsgMut<'a>> {
        self.cmsg.as_mut().map(CMsgMut)
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

pub(crate) fn space_of<T>() -> usize {
    unsafe { CMSG_SPACE(std::mem::size_of::<T>() as _) as _ }
}
