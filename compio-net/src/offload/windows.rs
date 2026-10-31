use compio_buf::IoBufMut;
use compio_io::ancillary::{AncillaryBuilder, CodecError};
use windows_sys::Win32::Networking::WinSock;

use crate::Socket;

#[cfg(feature = "windows-gro")]
pub(super) fn set_gro(socket: &Socket, enable: bool) -> std::io::Result<()> {
    let value: u32 = if enable { u16::MAX as u32 } else { 0 };
    unsafe {
        socket.set_socket_option(
            WinSock::IPPROTO_UDP,
            WinSock::UDP_RECV_MAX_COALESCED_SIZE,
            &value,
        )
    }
}

#[cfg(not(feature = "windows-gro"))]
pub(super) fn set_gro(_socket: &Socket, _enable: bool) -> std::io::Result<()> {
    Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
}

pub(super) fn max_gso_segments(socket: &Socket) -> usize {
    if unsafe { socket.get_socket_option::<i32>(WinSock::IPPROTO_UDP, WinSock::UDP_SEND_MSG_SIZE) }
        .is_ok()
    {
        512
    } else {
        1
    }
}

pub(super) fn push_segment_size<B: IoBufMut + ?Sized>(
    builder: &mut AncillaryBuilder<'_, B>,
    segment_size: u16,
) -> Result<(), CodecError> {
    builder.push(
        WinSock::IPPROTO_UDP,
        WinSock::UDP_SEND_MSG_SIZE,
        &(segment_size as u32),
    )
}

#[cfg(feature = "windows-gro")]
pub(super) unsafe fn segment_size(control: &[u8]) -> Result<Option<usize>, CodecError> {
    use compio_io::ancillary::AncillaryIter;
    const UDP_COALESCED_INFO: i32 = WinSock::UDP_COALESCED_INFO as i32;

    for cmsg in unsafe { AncillaryIter::new(control) } {
        if (cmsg.level(), cmsg.ty()) == (WinSock::IPPROTO_UDP, UDP_COALESCED_INFO) {
            return Ok(Some(cmsg.data::<u32>()? as usize));
        }
    }
    Ok(None)
}

#[cfg(not(feature = "windows-gro"))]
pub(super) unsafe fn segment_size(_control: &[u8]) -> Result<Option<usize>, CodecError> {
    Ok(None)
}
