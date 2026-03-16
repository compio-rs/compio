use std::mem::MaybeUninit;

use super::{AncillaryData, CodecError, copy_from_bytes, copy_to_bytes};

/// Marker trait to enable automatic `AncillaryData` implementation via
/// bytemuck.
///
/// Types that implement both [`bytemuck::NoUninit`] and this trait will
/// automatically implement [`AncillaryData`] using a simple byte-wise
/// encoding/decoding.
///
/// # Safety
///
/// This trait should only be implemented for types where a simple byte-wise
/// copy is a valid encoding/decoding strategy. The type must also implement
/// `bytemuck::NoUninit` to ensure it has no uninitialized bytes.
///
/// # Example
///
/// ```
/// use compio_io::ancillary::BytemuckMarker;
///
/// #[derive(Clone, Copy)]
/// #[repr(C)]
/// struct MyType {
///     value: u32,
/// }
///
/// unsafe impl bytemuck::NoUninit for MyType {}
/// impl BytemuckMarker for MyType {}
///
/// // Now MyType automatically implements AncillaryData
/// ```
pub trait BytemuckMarker {}

impl<T> AncillaryData for T
where
    T: bytemuck::NoUninit + BytemuckMarker,
{
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
            impl BytemuckMarker for $t {}
        )*
    };
}

impl_bytemuck_marker!(
    (),
    bool,
    char,
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

macro_rules! impl_bytemuck_marker_for_array {
    ($($N:expr),* $(,)?) => {
        $(
            impl<T> BytemuckMarker for [T; $N] where T: BytemuckMarker {}
        )*
    };
}

impl_bytemuck_marker_for_array!(
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26,
    27, 28, 29, 30, 31, 32, 48, 64, 96, 128, 256, 512,
);
