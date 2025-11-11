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

        if USED_OP.iter().all(|op| is_op_supported(*op)) {
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

#[cfg(io_uring)]
pub(crate) fn is_op_supported(code: u8) -> bool {
    #[cfg(feature = "once_cell_try")]
    use std::sync::OnceLock;

    use io_uring::Probe;
    #[cfg(not(feature = "once_cell_try"))]
    use once_cell::sync::OnceCell as OnceLock;

    static PROBE: OnceLock<Probe> = OnceLock::new();

    PROBE
        .get_or_try_init(|| {
            use io_uring::IoUring;

            let mut probe = Probe::new();

            IoUring::new(2)?.submitter().register_probe(&mut probe)?;

            std::io::Result::Ok(probe)
        })
        .map(|probe| probe.is_supported(code))
        .unwrap_or_default()
}
