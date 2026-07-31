//! [`tokio-console`] instrumentation.
//!
//! [`tokio-console`] collects its data through [`tracing`] spans and events
//! that follow a fixed naming convention. It is *not* tied to tokio's internals
//! in any way, so any executor emitting the same spans and events can be
//! observed with it.
//!
//! Enable the `console` feature to make this executor emit them:
//!
//! * every task gets a `runtime.spawn` span, entered while the task is polled,
//!   so that the console can compute poll counts, busy/idle/scheduled times and
//!   the poll time histogram;
//! * every waker operation emits a `runtime::waker` event, so that the console
//!   can compute waker counts and detect self-wakes and lost wakers.
//!
//! When the feature is disabled, all of this compiles down to nothing: the
//! types in this module become zero-sized and every method an empty inlined
//! function.
//!
//! # Usage
//!
//! `console-subscriber` refuses to run unless it can prove that the runtime is
//! instrumented, which for tokio means the `tokio_unstable` cfg. For other
//! runtimes it provides the `console_without_tokio_unstable` escape hatch, so a
//! binary observing compio needs:
//!
//! ```toml
//! # .cargo/config.toml
//! [build]
//! rustflags = ["--cfg", "console_without_tokio_unstable"]
//! ```
//!
//! Installing the subscriber is then all it takes. The `compio` crate
//! re-exports `console-subscriber` for the purpose, so that its version cannot
//! drift from the one this instrumentation was written against:
//!
//! ```ignore
//! compio::console_subscriber::init();
//! compio::runtime::Runtime::new().unwrap().block_on(async {
//!     // ...
//! });
//! ```
//!
//! # Limitations
//!
//! * The console's data model has one runtime per process, while compio is
//!   thread-per-core and has one executor per thread. The tasks of all of them
//!   are listed together; the `thread` field tells them apart.
//! * The blocking pool is part of the driver rather than the executor, so a
//!   task spawned by `spawn_blocking` is reported as an ordinary task waiting
//!   for an operation, and the time spent in the closure counts as idle.
//! * A `block_on` nested inside a task — a runtime built within another one —
//!   reports the two as separate tasks, but both of their spans are entered on
//!   the same stack. The console attributes the polls to the inner one for as
//!   long as that is the case.
//! * The resources tab stays empty: timers and in-flight operations are not
//!   instrumented yet.
//!
//! [`tokio-console`]: https://github.com/tokio-rs/console
//! [`tracing`]: https://docs.rs/tracing

#[cfg(not(feature = "console"))]
mod disabled;
#[cfg(feature = "console")]
mod enabled;

#[cfg(not(feature = "console"))]
pub(crate) use disabled::TaskSpan;
#[cfg(not(feature = "console"))]
pub use disabled::{SpawnMeta, instrument_block_on};
#[cfg(feature = "console")]
pub(crate) use enabled::TaskSpan;
#[cfg(feature = "console")]
pub use enabled::{SpawnMeta, instrument_block_on};

/// An operation on a task's waker, reported as a `runtime::waker` event.
///
/// Note that [`Waker::wake`](std::task::Waker::wake) does not call the `drop`
/// implementation, so the console counts [`Self::Wake`] as both a wake and a
/// drop. Emitting an additional [`Self::Drop`] for it would make the live waker
/// count (clones - drops) go negative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WakerOp {
    Clone,
    Drop,
    Wake,
    WakeByRef,
}

impl WakerOp {
    /// The `op` value of the event, as expected by the console.
    ///
    /// Only the enabled variant reports anything, so only it reads this.
    #[cfg(feature = "console")]
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Clone => "waker.clone",
            Self::Drop => "waker.drop",
            Self::Wake => "waker.wake",
            Self::WakeByRef => "waker.wake_by_ref",
        }
    }
}
