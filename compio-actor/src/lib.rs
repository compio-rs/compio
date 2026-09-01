//! An actor framework built for Compio.
//!
#![doc = include_str!("../README.md")]
#![deny(rustdoc::broken_intra_doc_links)]
#![warn(missing_docs)]

pub mod actor;
pub mod cluster;
pub mod mailbox;
pub mod process_group;
pub mod supervisor;

#[doc(inline)]
pub use actor::{Actor, ActorExit, ActorHandle, Handler, Message};
#[doc(inline)]
pub use cluster::Cluster;
#[doc(inline)]
pub use mailbox::{Broker, Call, Mailbox, Reply};
