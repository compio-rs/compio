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
    /// Get the underlying driver type
    #[cfg(fusion)]
    pub(crate) fn suggest() -> DriverType {
        use io_uring::opcode::*;

        // Add more opcodes here if used
        const USED_OP: &[u8] = &[
            Read::CODE,
            Readv::CODE,
            Write::CODE,
            Writev::CODE,
            Fsync::CODE,
            Accept::CODE,
            Connect::CODE,
            RecvMsg::CODE,
            SendMsg::CODE,
            AsyncCancel::CODE,
            OpenAt::CODE,
            Close::CODE,
            Shutdown::CODE,
        ];

        (|| {
            let uring = io_uring::IoUring::new(2)?;
            let mut probe = io_uring::Probe::new();
            uring.submitter().register_probe(&mut probe)?;
            if USED_OP.iter().all(|op| probe.is_supported(*op)) {
                std::io::Result::Ok(DriverType::IoUring)
            } else {
                Ok(DriverType::Poll)
            }
        })()
        .unwrap_or(DriverType::Poll) // Should we fail here?
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
