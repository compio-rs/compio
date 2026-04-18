//! Platform-specific and driver-specific utils & implementations used for other
//! modules in sys.

use crate::sys::prelude::*;

#[cfg(unix)]
mod_use![unix];

#[cfg(windows)]
mod_use![windows];

#[cfg(polling)]
mod_use![poll];

#[cfg(io_uring)]
mod_use![iour];

#[cfg(stub)]
mod_use![stub];
