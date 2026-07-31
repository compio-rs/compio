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
//!   the poll time histogram.
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
//! * The blocking pool is part of the driver rather than the executor, so a
//!   task spawned by `spawn_blocking` is reported as an ordinary task waiting
//!   for an operation, and the time spent in the closure counts as idle.
//! * The resources tab stays empty: timers and in-flight operations are not
//!   instrumented yet.
//! * A task's span is closed even when the thread is unwinding, or the console
//!   would show the task as running forever. The subscriber therefore runs
//!   during a panic, where a panic of its own aborts instead of unwinding.
//!
//! [`tokio-console`]: https://github.com/tokio-rs/console
//! [`tracing`]: https://docs.rs/tracing

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

pub use imp::SpawnMeta;
pub(crate) use imp::TaskSpan;
