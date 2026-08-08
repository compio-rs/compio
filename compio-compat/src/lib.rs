//! Runtime-compatibility layers for compio.
//!
//! This crate provides a compatibility layer for compio's runtime, allowing it
//! to be used with different underlying event loop implementations, e.g.,
//! `tokio` or `smol`.

#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]
#![doc(
    html_logo_url = "https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-bold.svg"
)]
#![doc(
    html_favicon_url = "https://github.com/compio-rs/compio-logo/raw/refs/heads/master/generated/colored-bold.svg"
)]

use std::{
    io,
    ops::Deref,
    task::{Context, Poll},
    time::Duration,
};

use compio_log::error;
use compio_runtime::{Runtime, SpawnMeta, console};
use mod_use::mod_use;

mod_use![sys];

/// A compatibility layer for [`Runtime`]. It is driven by the underlying
/// [`Adapter`].
pub struct RuntimeCompat<A> {
    runtime: A,
}

impl<A: Adapter> RuntimeCompat<A> {
    /// Creates a new [`RuntimeCompat`] with the given runtime.
    pub fn new(runtime: Runtime) -> io::Result<Self> {
        let runtime = A::new(runtime)?;
        Ok(Self { runtime })
    }

    /// Executes the given future on the runtime, driving it to completion.
    ///
    /// This is a plain `fn` so that the console can report the caller:
    /// `#[track_caller]` is a no-op on an `async fn`.
    #[track_caller]
    pub fn execute<F: Future>(&self, f: F) -> impl Future<Output = F::Output> {
        // The console has no kind of its own for this, and reports it the way
        // it reports a future the runtime blocks on, so the name is what tells
        // the two apart. Captured before the future, which is where the caller
        // is lost.
        let f = console::instrument_execute(SpawnMeta::capture().named("execute"), f);

        self.drive(f)
    }

    async fn drive<F: Future>(&self, f: F) -> F::Output {
        let waker = self.runtime.waker();
        let mut context = Context::from_waker(&waker);
        let mut future = std::pin::pin!(f);
        loop {
            if let Poll::Ready(result) = self.runtime.enter(|| future.as_mut().poll(&mut context)) {
                self.runtime.enter(|| self.runtime.run());
                return result;
            }

            let mut remaining_tasks = self.runtime.enter(|| self.runtime.run());

            remaining_tasks |= self.runtime.flush();

            let timeout = if remaining_tasks {
                Some(Duration::ZERO)
            } else {
                self.runtime.current_timeout()
            };

            match self.runtime.wait(timeout).await {
                Ok(_) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
                    ) => {}
                Err(e) => panic!("failed to wait for driver: {e:?}"),
            }

            if let Err(e) = self.runtime.clear() {
                error!("failed to clear notifier: {e:?}");
            }

            self.runtime.poll_with(Some(Duration::ZERO));
        }
    }
}

impl<A: Adapter> Deref for RuntimeCompat<A> {
    type Target = Runtime;

    fn deref(&self) -> &Self::Target {
        &self.runtime
    }
}
