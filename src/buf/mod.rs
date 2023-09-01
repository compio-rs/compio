//! Utilities for working with buffers.
//!
//! IOCP APIs require passing ownership of buffers to the runtime. The
//! crate defines [`IoBuf`] and [`IoBufMut`] traits which are implemented by buffer
//! types that respect the IOCP contract.

mod io_buf;
pub use io_buf::*;

mod slice;
pub use slice::*;

mod with_buf;
pub(crate) use with_buf::*;

mod buf_wrapper;
pub(crate) use buf_wrapper::*;

/// Trait to get the inner buffer of an operation or a result.
pub trait IntoInner {
    /// The inner type.
    type Inner;

    /// Get the inner buffer.
    fn into_inner(self) -> Self::Inner;
}

impl<T: IntoInner, O> IntoInner for crate::BufResult<O, T> {
    type Inner = crate::BufResult<O, T::Inner>;

    fn into_inner(self) -> Self::Inner {
        (self.0, self.1.into_inner())
    }
}
