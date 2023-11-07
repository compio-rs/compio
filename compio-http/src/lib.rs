//! A mid level HTTP services for [`hyper`].

#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

mod service;
pub use service::*;

mod backend;
pub use backend::*;

#[cfg(feature = "server")]
mod server;
#[cfg(feature = "server")]
pub use server::*;

mod stream;
pub use stream::*;
