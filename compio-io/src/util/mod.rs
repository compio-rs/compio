//! IO related utilities functions for ease of use.
mod copy;
pub use copy::*;

mod take;
pub use take::Take;

mod null;
pub use null::{Null, null};

mod repeat;
pub use repeat::{Repeat, repeat};

mod internal;
pub(crate) use internal::*;

pub mod split;
pub use split::Splittable;
