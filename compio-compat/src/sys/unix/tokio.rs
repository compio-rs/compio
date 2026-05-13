use std::{io, ops::Deref, time::Duration};

use compio_runtime::Runtime;
use tokio::io::{Interest, unix::AsyncFd};

use crate::{Adapter, sys::unix::UnixAdapter};

/// Adapter for `tokio` runtime.
pub struct TokioAdapter(AsyncFd<UnixAdapter>);

impl Adapter for TokioAdapter {
    fn new(runtime: Runtime) -> io::Result<Self> {
        Ok(Self(AsyncFd::with_interest(
            UnixAdapter::new(runtime)?,
            Interest::READABLE,
        )?))
    }

    async fn wait(&self, timeout: Option<Duration>) -> io::Result<()> {
        let fut = self.0.readable();
        let mut guard = if let Some(timeout) = timeout {
            tokio::time::timeout(timeout, fut).await??
        } else {
            fut.await?
        };
        guard.clear_ready();
        Ok(())
    }

    fn clear(&self) -> io::Result<()> {
        self.0.get_ref().clear()
    }
}

impl Deref for TokioAdapter {
    type Target = Runtime;

    fn deref(&self) -> &Self::Target {
        self.0.get_ref()
    }
}
