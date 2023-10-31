//! Filesystem manipulation operations.

#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]
#![warn(missing_docs)]

mod file;
pub use file::*;

#[cfg(feature = "runtime")]
mod open_options;
#[cfg(feature = "runtime")]
pub use open_options::*;

#[cfg(windows)]
pub mod named_pipe;

#[cfg(unix)]
pub mod pipe;
