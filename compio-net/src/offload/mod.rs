//! Ancillary-data helpers for UDP GSO (Generic Segmentation Offload) and GRO
//! (Generic Receive Offload).
//!
//! These work directly with the `control` buffers already accepted by
//! [`UdpSocket::send_msg`]/[`UdpSocket::recv_msg`] (and their
//! vectored/managed/multi variants) - no dedicated socket or buffer type is
//! needed. Pair them with [`UdpSocket::set_gro`] and
//! [`UdpSocket::max_gso_segments`].
//!
//! ## Platform-specific
//! * GSO ([`push_segment_size`]) is supported on Linux/Android and Windows.
//! * GRO ([`segment_size`]) is supported on Linux/Android, and on Windows only
//!   with the `windows-gro` feature (unverified - see its documentation).
//!   Elsewhere, [`push_segment_size`] is a no-op and [`segment_size`] always
//!   returns `Ok(None)`.
//!
//! [`UdpSocket::send_msg`]: crate::UdpSocket::send_msg
//! [`UdpSocket::recv_msg`]: crate::UdpSocket::recv_msg
//! [`UdpSocket::set_gro`]: crate::UdpSocket::set_gro
//! [`UdpSocket::max_gso_segments`]: crate::UdpSocket::max_gso_segments

use compio_buf::IoBufMut;
use compio_io::ancillary::{AncillaryBuilder, CodecError};

use crate::Socket;

cfg_select! {
    any(target_os = "linux", target_os = "android") => {
        #[path = "linux.rs"]
        mod sys;
    }
    windows => {
        #[path = "windows.rs"]
        mod sys;
    }
    _ => {
        #[path = "stub.rs"]
        mod sys;
    }
}

pub(crate) fn set_gro(socket: &Socket, enable: bool) -> std::io::Result<()> {
    sys::set_gro(socket, enable)
}

pub(crate) fn max_gso_segments(socket: &Socket) -> usize {
    sys::max_gso_segments(socket)
}

/// Attach a GSO control message requesting the kernel split the buffer of a
/// subsequent [`send_msg`] call into datagrams of `segment_size` bytes each,
/// up to [`UdpSocket::max_gso_segments`] segments.
///
/// [`send_msg`]: crate::UdpSocket::send_msg
/// [`UdpSocket::max_gso_segments`]: crate::UdpSocket::max_gso_segments
pub fn push_segment_size<B: IoBufMut + ?Sized>(
    builder: &mut AncillaryBuilder<'_, B>,
    segment_size: u16,
) -> Result<(), CodecError> {
    sys::push_segment_size(builder, segment_size)
}

/// Look for a GRO control message in `control`, as populated by
/// [`recv_msg`] after [`UdpSocket::set_gro`] was enabled, and return the
/// size of a single coalesced datagram if present.
///
/// Returns `Ok(None)` if no such control message is present, meaning
/// the whole received buffer is a single datagram.
///
/// # Safety
///
/// `control` must be exactly the (possibly empty) control buffer filled
/// by a `recv_msg` call - see [`AncillaryIter::new`].
///
/// [`recv_msg`]: crate::UdpSocket::recv_msg
/// [`UdpSocket::set_gro`]: crate::UdpSocket::set_gro
/// [`AncillaryIter::new`]: compio_io::ancillary::AncillaryIter::new
pub unsafe fn segment_size(control: &[u8]) -> Result<Option<usize>, CodecError> {
    unsafe { sys::segment_size(control) }
}
