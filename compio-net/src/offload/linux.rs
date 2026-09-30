use compio_buf::IoBufMut;
use compio_io::ancillary::{AncillaryBuilder, AncillaryIter, CodecError};

use crate::Socket;

pub(super) fn set_gro(socket: &Socket, enable: bool) -> std::io::Result<()> {
    unsafe { socket.set_socket_option(libc::SOL_UDP, libc::UDP_GRO, &(enable as libc::c_int)) }
}

pub(super) fn max_gso_segments(socket: &Socket) -> usize {
    if unsafe { socket.get_socket_option::<libc::c_int>(libc::SOL_UDP, libc::UDP_SEGMENT) }.is_ok()
    {
        32
    } else {
        1
    }
}

pub(super) fn push_segment_size<B: IoBufMut + ?Sized>(
    builder: &mut AncillaryBuilder<'_, B>,
    segment_size: u16,
) -> Result<(), CodecError> {
    builder.push(libc::SOL_UDP, libc::UDP_SEGMENT, &segment_size)
}

pub(super) unsafe fn segment_size(control: &[u8]) -> Result<Option<usize>, CodecError> {
    for cmsg in unsafe { AncillaryIter::new(control) } {
        if (cmsg.level(), cmsg.ty()) == (libc::SOL_UDP, libc::UDP_GRO) {
            return Ok(Some(cmsg.data::<libc::c_int>()? as usize));
        }
    }
    Ok(None)
}
