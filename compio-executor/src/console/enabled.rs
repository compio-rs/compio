//! The actual [`tokio-console`] instrumentation.
//!
//! [`tokio-console`]: https://github.com/tokio-rs/console

use std::{
    mem,
    panic::Location,
    sync::atomic::{AtomicU64, Ordering},
};

use tracing::Span;

use super::WakerOp;

/// Target of the task spans. The console accepts any target for spans named
/// `runtime.spawn`, but the target is displayed, so make it a useful one.
const TARGET: &str = "compio::task";

/// Id displayed by the console in its `ID` column.
///
/// This is deliberately *not* the executor's [`TaskId`], which is a slot index
/// and thus both reused and duplicated across the executors of a
/// thread-per-core application.
///
/// [`TaskId`]: crate::queue::TaskId
fn next_task_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);

    NEXT.fetch_add(1, Ordering::Relaxed)
}

std::thread_local! {
    /// Label of the current thread, to tell the tasks of the executors of a
    /// thread-per-core application apart.
    static THREAD: String = {
        let thread = std::thread::current();
        match thread.name() {
            Some(name) => name.to_owned(),
            None => format!("{:?}", thread.id()),
        }
    };
}

fn spawn_span(kind: &'static str, size: usize, loc: &'static Location<'static>) -> Span {
    fn build(
        kind: &'static str,
        size: usize,
        loc: &'static Location<'static>,
        thread: &str,
    ) -> Span {
        tracing::trace_span!(
            target: TARGET,
            // The console attributes polls of a task to the innermost task span
            // that is entered, so task spans must never be nested.
            parent: None,
            "runtime.spawn",
            kind = kind,
            task.id = next_task_id(),
            size.bytes = size,
            thread = thread,
            loc.file = loc.file(),
            loc.line = loc.line(),
            loc.col = loc.column(),
        )
    }

    // The label is gone once the thread tears its locals down, which a task
    // spawned from another destructor still runs after. Report it unlabelled
    // rather than panicking on the way out.
    THREAD
        .try_with(|thread| build(kind, size, loc, thread.as_str()))
        .unwrap_or_else(|_| build(kind, size, loc, ""))
}

fn waker_op(id: u64, op: WakerOp) {
    // `task.id` of a waker event is the *span* id of the task, which is how the
    // console looks the task up.
    tracing::trace!(target: "runtime::waker", op = op.as_str(), task.id = id);
}

/// Metadata of a spawned task, reported to the console.
///
/// The disabled variant of this is zero-sized, so nothing here may be
/// load-bearing outside of the console.
#[derive(Debug, Clone, Copy)]
pub struct SpawnMeta(&'static Location<'static>);

impl SpawnMeta {
    /// Capture the location of the caller.
    #[inline]
    #[track_caller]
    pub fn capture() -> Self {
        Self(Location::caller())
    }
}

/// The guard [`TaskSpan::enter`] returns, named the same in both variants so
/// that the parity assertions can reach it.
pub(crate) type EnterGuard<'a> = tracing::span::Entered<'a>;

/// The `runtime.spawn` span of a task.
#[derive(Debug)]
pub(crate) struct TaskSpan(Span);

impl TaskSpan {
    pub(crate) fn new<F>(meta: SpawnMeta) -> Self {
        Self(spawn_span("task", mem::size_of::<F>(), meta.0))
    }

    /// Enter the task span. The console measures the time the span is entered
    /// as the busy time of the task, and counts one poll per entry.
    #[inline]
    pub(crate) fn enter(&self) -> EnterGuard<'_> {
        self.0.enter()
    }

    /// Record a waker operation on this task.
    #[inline]
    pub(crate) fn waker_op(&self, op: WakerOp) {
        if let Some(id) = self.0.id() {
            waker_op(id.into_u64(), op);
        }
    }
}
