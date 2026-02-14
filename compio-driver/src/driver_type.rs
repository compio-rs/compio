/// Representing underlying driver type the fusion driver is using
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DriverType {
    /// Using `polling` driver
    Poll,
    /// Using `io-uring` driver
    IoUring,
    /// Using `iocp` driver
    IOCP,
}

impl DriverType {
    /// Suggest the driver type base on OpCode availability.
    ///
    /// This is used when the user doesn't specify a driver type, and the driver
    /// will choose the best one based on the supported OpCodes.
    #[cfg(fusion)]
    pub(crate) fn suggest(additional: crate::op::OpCodeFlag) -> DriverType {
        use crate::op::OpCodeFlag;

        let flags = additional | OpCodeFlag::basic();

        if flags.get_codes().all(crate::sys::is_op_supported) {
            DriverType::IoUring
        } else {
            DriverType::Poll
        }
    }

    /// Check if the current driver is `polling`
    pub fn is_polling(&self) -> bool {
        *self == DriverType::Poll
    }

    /// Check if the current driver is `io-uring`
    pub fn is_iouring(&self) -> bool {
        *self == DriverType::IoUring
    }

    /// Check if the current driver is `iocp`
    pub fn is_iocp(&self) -> bool {
        *self == DriverType::IOCP
    }
}
