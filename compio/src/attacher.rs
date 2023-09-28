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
pub struct Attacher {
    // Make it thread safe.
    once: OnceLock<()>,
    // Make it !Send & !Sync.
    _p: PhantomData<*mut ()>,
}

impl Attacher {
    pub const fn new() -> Self {
        Self {
            once: OnceLock::new(),
            _p: PhantomData,
        }
    }

    pub fn attach(&self, source: &impl AsRawFd) -> io::Result<()> {
        self.once.get_or_try_init(|| attach(source.as_raw_fd()))?;
        Ok(())
    }

    pub fn is_attached(&self) -> bool {
        self.once.get().is_some()
    }

    pub fn try_clone(&self, source: &impl AsRawFd) -> io::Result<Self> {
        if cfg!(target_os = "windows") {
            Ok(self.clone())
        } else {
            let new_self = Self::new();
            if self.is_attached() {
                new_self.attach(source)?;
            }
            Ok(new_self)
        }
    }
}
