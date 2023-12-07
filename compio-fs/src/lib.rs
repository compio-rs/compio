//! Filesystem manipulation operations.

#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![warn(missing_docs)]
#![cfg_attr(feature = "windows_by_handle", feature(windows_by_handle))]

mod file;
pub use file::*;

mod open_options;
pub use open_options::*;

mod metadata;
pub use metadata::*;

#[cfg(windows)]
pub mod named_pipe;

#[cfg(unix)]
pub mod pipe;
