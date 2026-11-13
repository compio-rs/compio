use compio_buf::IoBufMut;
use compio_io::ancillary::{AncillaryBuilder, CodecError};
use windows_sys::Win32::Networking::WinSock;

use crate::Socket;

cfg_select! {
    feature = "windows-gro" => {
        pub(crate) fn set_gro(socket: &Socket, enable: bool) -> std::io::Result<()> {
            let value: u32 = if enable { u16::MAX as u32 } else { 0 };
            unsafe {
                socket.set_socket_option(
                    WinSock::IPPROTO_UDP,
                    WinSock::UDP_RECV_MAX_COALESCED_SIZE,
                    &value,
                )
            }
        }

        pub(super) const GRO_CMSG: Option<(i32, i32)> =
            Some((WinSock::IPPROTO_UDP, WinSock::UDP_COALESCED_INFO as i32));
    }
    _ => {
        pub(crate) fn set_gro(_socket: &Socket, _enable: bool) -> std::io::Result<()> {
            Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
        }

        pub(super) const GRO_CMSG: Option<(i32, i32)> = None;
    }
}

pub(crate) fn max_gso_segments(socket: &Socket) -> usize {
    if unsafe { socket.get_socket_option::<i32>(WinSock::IPPROTO_UDP, WinSock::UDP_SEND_MSG_SIZE) }
        .is_ok()
    {
        // empirically found on Windows 11 x64
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
