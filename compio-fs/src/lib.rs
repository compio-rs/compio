//! Filesystem manipulation operations.

#![cfg_attr(feature = "allocator_api", feature(allocator_api))]

mod file;
pub use file::*;

mod open_options;
pub use open_options::*;
