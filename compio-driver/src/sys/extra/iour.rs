use io_uring::squeue::Flags;
use slotmap::DefaultKey;

/// Extra data for RawOp.
#[derive(Debug)]
pub(in crate::sys) struct Extra {
    sqe_flags: Flags,
    cqe_flags: u32,
    personality: Option<u16>,
    /// Slot of this op inside the `in_flight` map of the `io_uring` driver,
    /// set while the op is in flight.
    in_flight: Option<DefaultKey>,
}

pub(in crate::sys) use Extra as IourExtra;

impl Extra {
    pub fn new() -> Self {
        Self {
            sqe_flags: Flags::empty(),
            cqe_flags: 0,
            personality: None,
            in_flight: None,
        }
    }

    pub fn set_in_flight(&mut self, slot: DefaultKey) {
        debug_assert!(self.in_flight.is_none(), "op is already in flight");
        self.in_flight = Some(slot);
    }

    pub fn take_in_flight(&mut self) -> Option<DefaultKey> {
        self.in_flight.take()
    }

    pub fn set_personality(&mut self, personality: u16) {
        self.personality = Some(personality);
    }

    pub fn set_link(&mut self) {
        self.sqe_flags |= Flags::IO_LINK;
    }

    pub fn set_hardlink(&mut self) {
        self.sqe_flags |= Flags::IO_HARDLINK;
    }

    pub fn set_drain(&mut self) {
        self.sqe_flags |= Flags::IO_DRAIN;
    }

    pub fn get_personality(&self) -> Option<u16> {
        self.personality
    }

    pub fn get_sqe_flags(&self) -> Flags {
        self.sqe_flags
    }

    pub fn buffer_id(&self) -> Option<u16> {
        io_uring::cqueue::buffer_select(self.cqe_flags)
    }

    pub fn sock_nonempty(&self) -> bool {
        io_uring::cqueue::sock_nonempty(self.cqe_flags)
    }

    pub fn is_notification(&self) -> bool {
        io_uring::cqueue::notif(self.cqe_flags)
    }
}

#[allow(dead_code)]
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
            extra.cqe_flags = flag;
        }
    }
}
