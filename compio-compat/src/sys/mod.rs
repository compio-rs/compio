use std::{io, ops::Deref, time::Duration};

use compio_runtime::Runtime;
use mod_use::mod_use;

cfg_if::cfg_if! {
    if #[cfg(windows)] {
        mod_use![windows];
    } else if #[cfg(unix)] {
        mod_use![unix];
    } else {
        compile_error!("Unsupported platform");
    }
}

/// Adapter trait for different runtimes.
#[allow(async_fn_in_trait)]
pub trait Adapter: Sized + Deref<Target = Runtime> {
    /// Creates a new adapter with the given runtime.
    fn new(runtime: Runtime) -> io::Result<Self>;

    /// Waits for the runtime to be ready, with an optional timeout.
    async fn wait(&self, timeout: Option<Duration>) -> io::Result<()>;

    /// Clears the runtime's state after waiting.
    fn clear(&self) -> io::Result<()>;
}
