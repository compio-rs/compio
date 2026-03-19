//! Extension module for automatic [`AncillaryData`] implementation via
//! bytemuck.
//!
//! See [`BitwiseAncillaryData`] for details.

use std::mem::MaybeUninit;

pub use bytemuck::{Pod, Zeroable};

use super::{AncillaryData, CodecError, copy_from_bytes, copy_to_bytes};

/// Marker trait to enable automatic `AncillaryData` implementation via
/// bytemuck.
///
/// Types that implement this trait (which requires [`bytemuck::Pod`]) will
/// automatically implement [`AncillaryData`] using a simple byte-wise
/// encoding/decoding.
///
/// # Example
///
/// ```
/// use compio_io::ancillary::bytemuck_ext;
///
/// #[derive(Clone, Copy)]
/// #[repr(C)]
/// struct MyType {
///     value: u32,
/// }
///
/// unsafe impl bytemuck_ext::Zeroable for MyType {}
/// unsafe impl bytemuck_ext::Pod for MyType {}
/// impl bytemuck_ext::BitwiseAncillaryData for MyType {}
///
/// // Now MyType automatically implements AncillaryData
/// ```
pub trait BitwiseAncillaryData: Pod {}

impl<T: BitwiseAncillaryData> AncillaryData for T {
    fn encode(&self, buffer: &mut [MaybeUninit<u8>]) -> Result<(), CodecError> {
        unsafe { copy_to_bytes(self, buffer) }
    }

    fn decode(buffer: &[u8]) -> Result<Self, CodecError> {
        unsafe { copy_from_bytes(buffer) }
    }
}

macro_rules! impl_bytemuck_marker {
    ($($t:ty),* $(,)?) => {
        $(
            impl BitwiseAncillaryData for $t {}
        )*
    };
}

impl_bytemuck_marker!(
    (),
    u8,
    u16,
    u32,
    u64,
    u128,
    usize,
    i8,
    i16,
    i32,
    i64,
    i128,
    isize,
    f32,
    f64,
);

impl<T: BitwiseAncillaryData, const N: usize> BitwiseAncillaryData for [T; N] {}
