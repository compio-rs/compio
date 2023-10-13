//! Utilities for working with buffers.
//!
//! Completion APIs require passing ownership of buffers to the runtime. The
//! crate defines [`IoBuf`] and [`IoBufMut`] traits which are implemented by
//! buffer types that respect the safety contract.

#![cfg_attr(feature = "allocator_api", feature(allocator_api))]
#![cfg_attr(feature = "read_buf", feature(read_buf))]
#![cfg_attr(feature = "try_trait_v2", feature(try_trait_v2, try_trait_v2_residual))]
#![feature(return_position_impl_trait_in_trait)]
#![warn(missing_docs)]

#[cfg(feature = "arrayvec")]
pub use arrayvec;
#[cfg(feature = "bumpalo")]
pub use bumpalo;
#[cfg(feature = "bytes")]
pub use bytes;

mod io_slice;
pub use io_slice::*;

mod buf_result;
pub use buf_result::*;

mod io_buf;
pub use io_buf::*;

mod slice;
pub use slice::*;

mod iter;
pub use iter::*;

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
macro_rules! vec_alloc {
    ($t:ty, $a:ident) => {
        Vec<$t>
    };
}

#[cfg(feature = "allocator_api")]
#[macro_export]
#[doc(hidden)]
macro_rules! vec_alloc {
    ($t:ty, $a:ident) => {
        Vec<$t, $a>
    };
}

#[cfg(feature = "allocator_api")]
#[macro_export]
#[doc(hidden)]
macro_rules! box_alloc {
    ($t:ty, $a:ident) => {
        Box<$t, $a>
    };
}

#[cfg(not(feature = "allocator_api"))]
#[macro_export]
#[doc(hidden)]
macro_rules! box_alloc {
    ($t:ty, $a:ident) => {
        Box<$t>
    };
}
