//! No-op stand-ins used when the `console` feature is disabled.

use std::marker::PhantomData;

/// Metadata of a spawned task, reported to the console.
///
/// Zero-sized and inert unless the `console` feature is enabled.
#[derive(Debug, Clone, Copy)]
pub struct SpawnMeta;

impl SpawnMeta {
    /// Capture the location of the caller.
    #[inline(always)]
    pub fn capture() -> Self {
        Self
    }

    /// Do not report the task to the console at all.
    #[inline(always)]
    pub fn untracked() -> Self {
        Self
    }
}

/// The guard returned by [`TaskSpan::enter`].
///
/// The enabled variant measures the busy time of a task as the time its guard
/// is alive, so dropping it right away is a bug that this makes visible in both
/// configurations. It borrows the span for the same reason: the enabled guard
/// does, and code that compiles without the feature has to compile with it.
#[must_use = "the task span is exited as soon as this is dropped"]
pub(crate) struct Entered<'a>(PhantomData<&'a TaskSpan>);

/// The guard [`TaskSpan::enter`] returns, named the same in both variants so
/// that the parity assertions can reach it.
pub(crate) type EnterGuard<'a> = Entered<'a>;

/// The `runtime.spawn` span of a task.
#[derive(Debug)]
pub(crate) struct TaskSpan;

impl TaskSpan {
    #[inline(always)]
    #[expect(
        clippy::extra_unused_type_parameters,
        reason = "mirrors the enabled variant, which records the future's size"
    )]
    pub(crate) fn new<F>(_meta: SpawnMeta) -> Self {
        Self
    }

    #[inline(always)]
    pub(crate) fn enter(&self) -> EnterGuard<'_> {
        Entered(PhantomData)
    }

    #[inline(always)]
    pub(crate) fn waker_op(&self, _op: super::WakerOp) {}
}

/// Instrument a closure about to be handed to the blocking pool, so that it
/// shows up as a blocking task in the console.
///
/// This is a no-op unless the `console` feature is enabled.
#[inline(always)]
pub fn instrument_blocking<T, F: FnOnce() -> T>(_meta: SpawnMeta, f: F) -> impl FnOnce() -> T {
    f
}

/// Instrument a future blocked on by the runtime, so that it shows up as a
/// task in the console.
///
/// This is a no-op unless the `console` feature is enabled.
#[inline(always)]
pub fn instrument_block_on<F: Future>(fut: F) -> impl Future<Output = F::Output> {
    fut
}
