#[cfg(feature = "once_cell_try")]
use std::sync::OnceLock;
use std::{
    io,
    ops::{Deref, DerefMut},
};

use compio_buf::IntoInner;
use compio_driver::AsRawFd;
#[cfg(not(feature = "once_cell_try"))]
use once_cell::sync::OnceCell as OnceLock;

use crate::Runtime;

/// Attach a handle to the driver of current thread.
///
/// A handle can and only can attach once to one driver. The attacher will check
/// if it is attached to the current driver.
#[derive(Debug, Clone)]
pub struct Attacher<S> {
    source: S,
    // Make it thread safe.
    once: OnceLock<()>,
}

impl<S: AsRawFd> Attacher<S> {
    /// Create [`Attacher`]. It tries to attach the source, and will return
    /// [`Err`] if it fails.
    pub fn new(source: S) -> io::Result<Self> {
        let this = Self {
            source,
            once: OnceLock::new(),
        };
        this.attach()?;
        Ok(this)
    }

    /// Attach the source. This method could be called many times, but if the
    /// action fails, it will try to attach the source during each call.
    fn attach(&self) -> io::Result<()> {
        let r = Runtime::current();
        let inner = r.inner();
        self.once
            .get_or_try_init(|| inner.attach(self.source.as_raw_fd()))?;
        Ok(())
    }
}

impl<S> IntoInner for Attacher<S> {
    type Inner = S;

    fn into_inner(self) -> Self::Inner {
        self.source
    }
}

impl<S> Deref for Attacher<S> {
    type Target = S;

    fn deref(&self) -> &Self::Target {
        &self.source
    }
}

impl<S> DerefMut for Attacher<S> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.source
    }
}
