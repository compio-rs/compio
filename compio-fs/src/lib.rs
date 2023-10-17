//! Filesystem manipulation operations.

#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![warn(missing_docs)]

mod file;
pub use file::*;

mod open_options;
pub use open_options::*;

#[cfg(windows)]
pub mod named_pipe;
