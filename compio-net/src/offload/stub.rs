use compio_buf::IoBufMut;
use compio_io::ancillary::{AncillaryBuilder, CodecError};

use crate::Socket;

pub(super) fn set_gro(_socket: &Socket, _enable: bool) -> std::io::Result<()> {
    Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
}

pub(super) fn max_gso_segments(_socket: &Socket) -> usize {
    1
}

pub(super) fn push_segment_size<B: IoBufMut + ?Sized>(
    _builder: &mut AncillaryBuilder<'_, B>,
    _segment_size: u16,
) -> Result<(), CodecError> {
    Ok(())
}

pub(super) unsafe fn segment_size(_control: &[u8]) -> Result<Option<usize>, CodecError> {
    Ok(None)
}
