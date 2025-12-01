//! Utilities for working with buffers.
//!
//! Completion APIs require passing ownership of buffers to the runtime. The
//! crate defines [`IoBuf`] and [`IoBufMut`] traits which are implemented by
//! buffer types that respect the safety contract.

#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(feature = "allocator_api", feature(allocator_api))]
#![cfg_attr(feature = "read_buf", feature(read_buf, core_io_borrowed_buf))]
#![cfg_attr(feature = "try_trait_v2", feature(try_trait_v2, try_trait_v2_residual))]
#![warn(missing_docs)]

#[cfg(feature = "arrayvec")]
pub use arrayvec;
#[cfg(feature = "bumpalo")]
pub use bumpalo;
#[cfg(feature = "bytes")]
pub use bytes;

mod io_buffer;
pub use io_buffer::*;

mod io_buf;
pub use io_buf::*;

mod io_vec_buf;
pub use io_vec_buf::*;

mod buf_result;
pub use buf_result::*;

mod slice;
pub use slice::*;

mod uninit;
pub use uninit::*;

/// Trait to get the inner buffer of an operation or a result.
pub trait IntoInner {
    /// The inner type.
    type Inner;

    /// Get the inner buffer.
    fn into_inner(self) -> Self::Inner;
}

#[cfg(not(feature = "allocator_api"))]
#[macro_export]
#[doc(hidden)]
macro_rules! t_alloc {
    ($b:tt, $t:ty, $a:ident) => {
        $b<$t>
    };
}

#[cfg(feature = "allocator_api")]
#[macro_export]
#[doc(hidden)]
macro_rules! t_alloc {
    ($b:tt, $t:ty, $a:ident) => {
        $b<$t, $a>
    };
}
