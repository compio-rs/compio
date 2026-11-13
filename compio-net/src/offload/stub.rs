use compio_buf::IoBufMut;
use compio_io::ancillary::{AncillaryBuilder, CodecError};

use crate::Socket;

pub(crate) fn set_gro(_socket: &Socket, _enable: bool) -> std::io::Result<()> {
    Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
}

pub(crate) fn max_gso_segments(_socket: &Socket) -> usize {
    1
}

pub(super) fn push_segment_size<B: IoBufMut + ?Sized>(
    _builder: &mut AncillaryBuilder<'_, B>,
    _segment_size: u16,
) -> Result<(), CodecError> {
    Ok(())
}

pub(super) const GRO_CMSG: Option<(i32, i32)> = None;
