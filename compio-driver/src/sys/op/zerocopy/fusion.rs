use rustix::net::SendFlags;

use super::{iour, poll};

crate::macros::fuse_op! {
    <T: IoBuf, S: AsFd> SendZc(fd: S, buffer: T, flags: SendFlags);
    <T: IoVectoredBuf, S: AsFd> SendVectoredZc(fd: S, buffer: T, flags: SendFlags);
    <T: IoBuf, S: AsFd> SendToZc(fd: S, buffer: T, addr: SockAddr, flags: SendFlags);
    <T: IoVectoredBuf, S: AsFd> SendToVectoredZc(fd: S, buffer: T, addr: SockAddr, flags: SendFlags);
    <T: IoVectoredBuf, C: IoBuf, S: AsFd> SendMsgZc(fd: S, buffer: T, control: C, addr: Option<SockAddr>, flags: SendFlags);
}
