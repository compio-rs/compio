#[cfg(feature = "once_cell_try")]
use std::sync::OnceLock;
use std::{io, marker::PhantomData};

#[cfg(not(feature = "once_cell_try"))]
use once_cell::sync::OnceCell as OnceLock;

use crate::{driver::AsRawFd, task::attach};

/// Attach a handle to the driver of current thread.
///
/// A handle can and only can attach once to one driver. However, the handle
/// itself is Send & Sync. We mark it !Send & !Sync to warn users, making them
/// ensure that they are using it in the correct thread.
#[derive(Debug, Clone)]
pub struct Attacher {}

impl Attacher {
    pub const fn new() -> Self {
        Self {}
    }

    pub fn attach(&self, source: &impl AsRawFd) -> io::Result<()> {
        attach(source.as_raw_fd())
    }
}
