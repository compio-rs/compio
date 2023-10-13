//! # Compio IO
#![feature(async_fn_in_trait)] // Remove this when AFIT is stable

mod read;
mod util;
mod write;

pub(crate) use std::io::Result as IoResult;

pub use read::*;
pub use write::*;
