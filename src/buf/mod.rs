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
