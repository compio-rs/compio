#[cfg(feature = "once_cell_try")]
use std::sync::OnceLock;
use std::{io, marker::PhantomData};

use compio_driver::AsRawFd;
#[cfg(not(feature = "once_cell_try"))]
use once_cell::sync::OnceCell as OnceLock;

use crate::attach;

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
    /// Create [`Attacher`].
    pub const fn new() -> Self {
        Self {
            once: OnceLock::new(),
            _p: PhantomData,
        }
    }

    /// Attach the source. This method could be called many times, but if the
    /// action fails, the error will only return once.
    pub fn attach(&self, source: &impl AsRawFd) -> io::Result<()> {
        self.once.get_or_try_init(|| attach(source.as_raw_fd()))?;
        Ok(())
    }

    /// Check if [`attach`] has been called.
    pub fn is_attached(&self) -> bool {
        self.once.get().is_some()
    }

    /// Try clone self with the cloned source. The attach state will be
    /// reserved.
    ///
    /// ## Platform specific
    /// * io-uring/polling: it will try to attach in the current thread if
    ///   needed.
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
