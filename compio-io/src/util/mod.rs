//! IO related utilities functions for ease of use.

mod take;
pub use take::Take;

mod null;
pub use null::{null, Null};

mod internal;
pub(crate) use internal::*;


