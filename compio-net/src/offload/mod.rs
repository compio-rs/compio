use compio_buf::IoBufMut;
use compio_io::ancillary::{AncillaryBuilder, AncillaryIter, CodecError, ancillary_space};

cfg_select! {
    any(target_os = "linux", target_os = "android") => {
        mod linux;
        use linux as imp;
    }
    windows => {
        mod windows;
        use windows as imp;
    }
    _ => {
        mod stub;
        use stub as imp;
    }
}

pub(crate) use imp::{max_gso_segments, set_gro};

/// Attach a GSO (Generic Segmentation Offload) control message requesting the
/// kernel split the buffer of a subsequent [`send_msg`] call into datagrams of
/// `segment_size` bytes each, up to [`UdpSocket::max_gso_segments`] segments.
///
/// ## Platform-specific
/// * Only Linux/Android and Windows are supported. Elsewhere this is a no-op.
///
/// [`send_msg`]: crate::UdpSocket::send_msg
/// [`UdpSocket::max_gso_segments`]: crate::UdpSocket::max_gso_segments
pub fn push_gso_segment_size<B: IoBufMut + ?Sized>(
    builder: &mut AncillaryBuilder<'_, B>,
    segment_size: u16,
) -> Result<(), CodecError> {
    imp::push_segment_size(builder, segment_size)
}

/// Look for a GRO (Generic Receive Offload) control message in `control`, as
/// populated by [`recv_msg`] after [`UdpSocket::set_gro`] was enabled, and
/// return the size of a single coalesced datagram if present.
///
/// Returns `Ok(None)` if no such control message is present, meaning
/// the whole received buffer is a single datagram.
///
/// ## Platform-specific
/// * Only Linux/Android and, with the `windows-gro` feature, Windows are
///   supported. Elsewhere this always returns `Ok(None)`.
///
/// # Panics
///
/// Panics if `control` is not aligned for a control message header.
///
/// # Safety
///
/// `control` must be exactly the (possibly empty) control buffer filled
/// by a `recv_msg` call.
///
/// [`recv_msg`]: crate::UdpSocket::recv_msg
/// [`UdpSocket::set_gro`]: crate::UdpSocket::set_gro
pub unsafe fn gro_segment_size(control: &[u8]) -> Result<Option<usize>, CodecError> {
    let Some((level, ty)) = imp::GRO_CMSG else {
        return Ok(None);
    };
    // `AncillaryIter::new` panics on buffers shorter than a cmsg header, e.g.
    // when no cmsg was received
    if control.len() < ancillary_space::<u32>() {
        return Ok(None);
    }
    for cmsg in unsafe { AncillaryIter::new(control) } {
        if (cmsg.level(), cmsg.ty()) == (level, ty) {
            // a `c_int` on Linux and a `DWORD` on Windows, both 4 bytes
            return Ok(Some(cmsg.data::<u32>()? as usize));
        }
    }
    Ok(None)
}
