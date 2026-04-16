bitflags::bitflags! {
    /// Flags for operations.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct OpCodeFlag: u32 {
        /// Detect `Read` OpCode
        const Read = 1 << 0;
        /// Detect `Readv` OpCode
        const Readv = 1 << 1;
        /// Detect `Write` OpCode
        const Write = 1 << 2;
        /// Detect `Writev` OpCode
        const Writev = 1 << 3;
        /// Detect `Fsync` OpCode
        const Fsync = 1 << 4;
        /// Detect `Accept` OpCode
        const Accept = 1 << 5;
        /// Detect `Connect` OpCode
        const Connect = 1 << 6;
        /// Detect `Recv` OpCode
        const Recv = 1 << 7;
        /// Detect `Send` OpCode
        const Send = 1 << 8;
        /// Detect `RecvMsg` OpCode
        const RecvMsg = 1 << 9;
        /// Detect `SendMsg` OpCode
        const SendMsg = 1 << 10;
        /// Detect `AsyncCancel` OpCode
        const AsyncCancel = 1 << 11;
        /// Detect `OpenAt` OpCode
        const OpenAt = 1 << 12;
        /// Detect `Close` OpCode
        const Close = 1 << 13;
        /// Detect `Splice` OpCode
        const Splice = 1 << 14;
        /// Detect `Shutdown` OpCode
        const Shutdown = 1 << 15;
        /// Detect `PollAdd` OpCode
        const PollAdd = 1 << 16;
    }
}

impl OpCodeFlag {
    /// Get the [`OpCodeFlag`] corresponds to basic OpCodes that are commonly
    /// used.
    pub fn basic() -> Self {
        OpCodeFlag::Read
            | OpCodeFlag::Readv
            | OpCodeFlag::Write
            | OpCodeFlag::Writev
            | OpCodeFlag::Fsync
            | OpCodeFlag::Accept
            | OpCodeFlag::Connect
            | OpCodeFlag::Recv
            | OpCodeFlag::Send
            | OpCodeFlag::RecvMsg
            | OpCodeFlag::SendMsg
            | OpCodeFlag::PollAdd
    }
}

#[cfg(io_uring)]
impl OpCodeFlag {
    pub(crate) fn get_codes(self) -> impl Iterator<Item = u8> {
        use io_uring::opcode::*;

        self.iter().map(|flag| match flag {
            OpCodeFlag::Read => Read::CODE,
            OpCodeFlag::Readv => Readv::CODE,
            OpCodeFlag::Write => Write::CODE,
            OpCodeFlag::Writev => Writev::CODE,
            OpCodeFlag::Fsync => Fsync::CODE,
            OpCodeFlag::Accept => Accept::CODE,
            OpCodeFlag::Connect => Connect::CODE,
            OpCodeFlag::Recv => Recv::CODE,
            OpCodeFlag::Send => Send::CODE,
            OpCodeFlag::RecvMsg => RecvMsg::CODE,
            OpCodeFlag::SendMsg => SendMsg::CODE,
            OpCodeFlag::AsyncCancel => AsyncCancel::CODE,
            OpCodeFlag::OpenAt => OpenAt::CODE,
            OpCodeFlag::Close => Close::CODE,
            OpCodeFlag::Splice => Splice::CODE,
            OpCodeFlag::Shutdown => Shutdown::CODE,
            OpCodeFlag::PollAdd => PollAdd::CODE,
            unknown => unreachable!("Unknown OpCodeFlag specified: {unknown:?}"),
        })
    }
}
