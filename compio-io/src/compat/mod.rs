//! Compat wrappers for interop with other crates.

mod sync_stream;
pub use sync_stream::*;

mod async_stream;
pub use async_stream::*;
