use std::{io, ops::Deref, time::Duration};

use async_io::Async;
use compio_runtime::Runtime;
use futures_util::FutureExt;

use crate::{Adapter, sys::unix::UnixAdapter};

/// Adapter for general runtime. It is driven by `async-io`.
pub struct FuturesAdapter(Async<UnixAdapter>);

impl Adapter for FuturesAdapter {
    fn new(runtime: Runtime) -> io::Result<Self> {
        Ok(Self(Async::new_nonblocking(UnixAdapter::new(runtime)?)?))
    }

    async fn wait(&self, timeout: Option<Duration>) -> io::Result<()> {
        let fut = self.0.readable();
        if let Some(timeout) = timeout {
            let timer = async_io::Timer::after(timeout);
            futures_util::select! {
                res = fut.fuse() => res,
                _ = timer.fuse() => Err(io::ErrorKind::TimedOut.into()),
            }
        } else {
            fut.await
        }
    }

    fn clear(&self) -> io::Result<()> {
        self.0.get_ref().clear()
    }
}

impl Deref for FuturesAdapter {
    type Target = Runtime;

    fn deref(&self) -> &Self::Target {
        self.0.get_ref()
    }
}
