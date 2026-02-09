/// Extra data for RawOp.
pub struct Extra {
    flags: u32,
    personality: Option<u16>,
}

impl Extra {
    pub(crate) fn new() -> Self {
        Self {
            flags: 0,
            personality: None,
        }
    }

    pub(crate) fn set_personality(&mut self, personality: u16) {
        self.personality = Some(personality);
    }

    pub(crate) fn get_personality(&self) -> Option<u16> {
        self.personality
    }

    pub(crate) fn buffer_id(&self) -> Option<u16> {
        io_uring::cqueue::buffer_select(self.flags)
    }
}

#[cfg(not(fusion))]
impl crate::sys::Extra {
    pub(crate) fn is_iour(&self) -> bool {
        true
    }

    pub(in crate::sys) fn try_as_iour(&self) -> Option<&Extra> {
        Some(&self.0)
    }

    pub(in crate::sys) fn try_as_iour_mut(&mut self) -> Option<&mut Extra> {
        Some(&mut self.0)
    }
}

#[allow(dead_code)]
impl crate::sys::Extra {
    pub(in crate::sys) fn as_iour(&self) -> &Extra {
        self.try_as_iour()
            .expect("Current driver is not `io_uring`")
    }

    pub(in crate::sys) fn as_iour_mut(&mut self) -> &mut Extra {
        self.try_as_iour_mut()
            .expect("Current driver is not `io_uring`")
    }

    pub(crate) fn set_flags(&mut self, flag: u32) {
        if let Some(extra) = self.try_as_iour_mut() {
            extra.flags = flag;
        }
    }
}
