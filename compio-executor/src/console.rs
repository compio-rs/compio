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
//!   can compute waker counts and detect self-wakes and lost wakers;
//! * a closure handed to the blocking pool gets such a span too, entered around
//!   the closure instead of around a poll, so that the time spent in it is
//!   reported as busy time rather than as idle time.
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
//! Depending on `console-subscriber` and installing it is then all it takes:
//!
//! ```ignore
//! console_subscriber::init();
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
//! * The subscriber has to be the global default, which
//!   `console_subscriber::init` makes it. A span carries the subscriber it was
//!   created with, but an event goes to whichever one is current on the thread
//!   emitting it, so a thread-local subscriber misses the waker operations
//!   other threads perform. Wakers cross threads routinely — that is what
//!   waking a task from another executor is — and the clone and drop counts of
//!   one that does no longer balance, leaving the console to report a lost
//!   waker that is not lost.
//! * A `block_on` nested inside a task — a runtime built within another one —
//!   reports the two as separate tasks, but both of their spans are entered on
//!   the same stack. The console attributes the polls to the inner one for as
//!   long as that is the case.
//! * A blocking task has no waker operations, since it is a closure rather than
//!   a future. The console knows this from its `kind` and does not report a
//!   lost waker for it.
//! * A task spawned by an `async fn` is attributed to that function rather than
//!   to its caller, since [`#[track_caller]`][async-track-caller] is a no-op on
//!   `async fn`s and [`SpawnMeta`] therefore cannot be forwarded through them.
//!   The ones compio spawns itself are named to make up for it. A crate willing
//!   to build on nightly can lift this for its own `async fn`s with the
//!   `async_fn_track_caller` feature, which attributes them to the `.await` of
//!   the future they return.
//! * The resources tab stays empty: timers and in-flight operations are not
//!   instrumented yet.
//! * A task's span is closed even when the thread is unwinding, or the console
//!   would show the task as running forever. The subscriber therefore runs
//!   during a panic, where a panic of its own aborts instead of unwinding.
//!
//! [`tokio-console`]: https://github.com/tokio-rs/console
//! [`tracing`]: https://docs.rs/tracing
//! [async-track-caller]: https://github.com/rust-lang/rust/issues/110011

cfg_select! {
    feature = "console" => {
        mod enabled;
        use enabled as imp;
    }
    _ => {
        mod disabled;
        use disabled as imp;
    }
}

pub(crate) use imp::TaskSpan;
pub use imp::{SpawnMeta, instrument_block_on, instrument_blocking, instrument_execute};

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
/// Assertions that the two variants above present the same surface.
///
/// Only one of them is ever compiled, and the one compiled by default is the
/// one nearly every build uses: a difference between the two shows up as a
/// build failure for whoever turns the feature on, long after the code that
/// assumed the other shape was written.
///
/// Coercing each item to a function pointer pins its whole signature, and
/// naming [`EnterGuard`] with a lifetime pins the shape of the guard: the
/// enabled one borrows the span, so a disabled one that owns itself, and would
/// let code outlive the span it is timing, does not have a lifetime to name.
#[cfg(test)]
mod parity {
    use std::{fmt::Debug, future::Future};

    use super::{imp::EnterGuard, *};

    const _: fn() -> SpawnMeta = SpawnMeta::capture;
    const _: fn(SpawnMeta, &'static str) -> SpawnMeta = SpawnMeta::named;
    const _: fn() -> SpawnMeta = SpawnMeta::untracked;

    const _: fn(SpawnMeta) -> TaskSpan = TaskSpan::new::<()>;
    const _: for<'a> fn(&'a TaskSpan) -> EnterGuard<'a> = TaskSpan::enter;
    const _: fn(&TaskSpan, WakerOp) = TaskSpan::waker_op;

    /// [`SpawnMeta`] is copied out of a spawn call rather than moved, and
    /// reaches the dispatcher's threads through its channel.
    const fn meta<T: Copy + Send + Sync + Unpin + Debug + 'static>() {}
    const _: () = meta::<SpawnMeta>();

    /// [`TaskSpan`] sits in the task header, which threads share.
    const fn span<T: Send + Sync + Debug>() {}
    const _: () = span::<TaskSpan>();

    /// The wrappers return `impl Trait`, so pin them by use instead.
    #[test]
    fn the_wrappers_pass_their_argument_through() {
        assert_eq!(instrument_blocking(SpawnMeta::untracked(), || 1u8)(), 1);

        let fut = instrument_block_on(SpawnMeta::untracked(), std::future::ready(1u8));
        let _: &dyn Future<Output = u8> = &fut;

        let fut = instrument_execute(SpawnMeta::untracked(), std::future::ready(1u8));
        let _: &dyn Future<Output = u8> = &fut;
    }
}
