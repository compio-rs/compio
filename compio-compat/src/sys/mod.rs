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

#[allow(async_fn_in_trait)]
pub trait Adapter: Sized + Deref<Target = Runtime> {
    fn new(runtime: Runtime) -> io::Result<Self>;

    async fn wait(&self, timeout: Option<Duration>) -> io::Result<()>;

    fn clear(&self) -> io::Result<()>;
}
