use super::{iour, poll};

crate::macros::fuse_op! {
    <T: IoBuf, S: AsFd> SendZc(fd: S, buffer: T, flags: i32);
    <T: IoVectoredBuf, S: AsFd> SendVectoredZc(fd: S, buffer: T, flags: i32);
    <T: IoBuf, S: AsFd> SendToZc(fd: S, buffer: T, addr: SockAddr, flags: i32);
    <T: IoVectoredBuf, S: AsFd> SendToVectoredZc(fd: S, buffer: T, addr: SockAddr, flags: i32);
    <T: IoVectoredBuf, C: IoBuf, S: AsFd> SendMsgZc(fd: S, buffer: T, control: C, addr: Option<SockAddr>, flags: i32);
}
