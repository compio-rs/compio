//! A mid level HTTP services for [`hyper`].

#![warn(missing_docs)]
#![cfg_attr(feature = "read_buf", feature(read_buf, core_io_borrowed_buf))]
#![cfg_attr(docsrs, feature(doc_cfg, doc_auto_cfg))]

mod service;
pub use service::*;

mod backend;
pub use backend::*;

mod stream;
pub use stream::*;
