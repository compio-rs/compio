use std::io;
#[cfg(feature = "once_cell_try")]
use std::sync::OnceLock;

#[cfg(not(feature = "once_cell_try"))]
use once_cell::sync::OnceCell as OnceLock;

use crate::{driver::AsRawFd, task::RUNTIME};

#[derive(Debug)]
pub struct Attacher {
    once: OnceLock<()>,
}

impl Attacher {
    pub const fn new() -> Self {
        Self {
            once: OnceLock::new(),
        }
    }

    pub fn attach(&self, source: &impl AsRawFd) -> io::Result<()> {
        self.once
            .get_or_try_init(|| RUNTIME.with(|runtime| runtime.attach(source.as_raw_fd())))?;
        Ok(())
    }
}
