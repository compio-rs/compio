//! Compat wrappers for interop with other crates.

mod sync_stream;
pub use sync_stream::*;

mod async_stream;
pub use async_stream::*;

#[cfg(feature = "tokio")]
mod tokio_io;

mod waker_array;
pub use waker_array::*;
