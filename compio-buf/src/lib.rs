#![cfg_attr(feature = "allocator_api", feature(allocator_api))]
#![cfg_attr(feature = "read_buf", feature(read_buf))]

//! Utilities for working with buffers.
//!
//! Completion APIs require passing ownership of buffers to the runtime. The
//! crate defines [`IoBuf`] and [`IoBufMut`] traits which are implemented by
//! buffer types that respect the safety contract.

#[cfg(feature = "arrayvec")]
pub use arrayvec;
#[cfg(feature = "bumpalo")]
pub use bumpalo;
#[cfg(feature = "bytes")]
pub use bytes;

mod io_buf;
pub use io_buf::*;

mod slice;
pub use slice::*;

mod with_buf;
pub use with_buf::*;

mod buf_wrapper;
pub use buf_wrapper::*;

/// Trait to get the inner buffer of an operation or a result.
pub trait IntoInner {
    /// The inner type.
    type Inner;

    /// Get the inner buffer.
    fn into_inner(self) -> Self::Inner;
}

#[cfg(not(feature = "allocator_api"))]
#[macro_export]
macro_rules! vec_alloc {
    ($t:ident, $a:ident) => {
        Vec<$t>
    };
}

#[cfg(feature = "allocator_api")]
#[macro_export]
macro_rules! vec_alloc {
    ($t:ident, $a:ident) => {
        Vec<$t, $a>
    };
}

/// A specialized `Result` type for operations with buffers.
///
/// This type is used as a return value for asynchronous compio methods that
/// require passing ownership of a buffer to the runtime. When the operation
/// completes, the buffer is returned no matter if the operation completed
/// successfully.
pub type BufResult<T, B> = (std::io::Result<T>, B);

impl<T: IntoInner, O> IntoInner for BufResult<O, T> {
    type Inner = crate::BufResult<O, T::Inner>;

    fn into_inner(self) -> Self::Inner {
        (self.0, self.1.into_inner())
    }
}
