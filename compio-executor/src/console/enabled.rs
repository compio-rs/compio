//! The actual [`tokio-console`] instrumentation.
//!
//! [`tokio-console`]: https://github.com/tokio-rs/console

use std::{
    mem,
    panic::Location,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll, RawWaker, RawWakerVTable, Waker},
};

use tracing::Span;

use super::WakerOp;

/// Target of the task spans. The console accepts any target for spans named
/// `runtime.spawn`, but the target is displayed, so make it a useful one.
const TARGET: &str = "compio::task";

/// What a task span reports the task as being.
///
/// The console considers a task whose kind is not [`Self::Task`] to not be
/// driven by a future, and skips the lints that only make sense for one: the
/// self-wake ratio, the lost waker, the never-yielded and the large future
/// ones.
#[derive(Debug, Clone, Copy)]
enum TaskKind {
    Task,
    Blocking,
    BlockOn,
}

impl TaskKind {
    /// The `kind` value of the span, as expected by the console.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::Blocking => "blocking",
            Self::BlockOn => "block_on",
        }
    }
}

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

fn spawn_span(
    kind: TaskKind,
    size: usize,
    loc: &'static Location<'static>,
    name: Option<&'static str>,
) -> Span {
    fn build(
        kind: TaskKind,
        size: usize,
        loc: &'static Location<'static>,
        name: Option<&'static str>,
        thread: &str,
    ) -> Span {
        tracing::trace_span!(
            target: TARGET,
            // The console attributes polls of a task to the innermost task span
            // that is entered, so task spans must never be nested.
            parent: None,
            "runtime.spawn",
            kind = kind.as_str(),
            task.id = next_task_id(),
            // The console gives this field a column of its own, and leaves it
            // empty for the tasks that do not have it.
            task.name = name,
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
        .try_with(|thread| build(kind, size, loc, name, thread.as_str()))
        .unwrap_or_else(|_| build(kind, size, loc, name, ""))
}

fn waker_op(id: u64, op: WakerOp) {
    // `task.id` of a waker event is the *span* id of the task, which is how the
    // console looks the task up.
    tracing::trace!(target: "runtime::waker", op = op.as_str(), task.id = id);
}

/// Metadata of a spawned task, reported to the console.
///
/// `None` for a task that is not reported at all. The disabled variant of this
/// is zero-sized, so nothing here may be load-bearing outside of the console.
#[derive(Debug, Clone, Copy)]
pub struct SpawnMeta(Option<Reported>);

/// What is reported about a task, for the tasks that are reported at all.
#[derive(Debug, Clone, Copy)]
struct Reported {
    loc: &'static Location<'static>,
    name: Option<&'static str>,
}

impl SpawnMeta {
    /// Capture the location of the caller.
    #[inline]
    #[track_caller]
    pub fn capture() -> Self {
        Self(Some(Reported {
            loc: Location::caller(),
            name: None,
        }))
    }

    /// Name the task, which the console displays in a column of its own.
    ///
    /// This is worth doing for the tasks a user did not spawn themselves, since
    /// the location of those points into compio rather than into their code.
    #[inline]
    pub fn named(self, name: &'static str) -> Self {
        Self(self.0.map(|it| Reported {
            name: Some(name),
            ..it
        }))
    }

    /// Do not report the task to the console at all.
    ///
    /// This is for tasks that only wrap work reported by something else, like
    /// the future waiting for a blocking closure: reporting both would count
    /// the same work twice, and hide the interesting one behind a wrapper that
    /// is idle the whole time.
    #[inline]
    pub fn untracked() -> Self {
        Self(None)
    }

    /// The span of the task, or a disabled span if it is not to be reported.
    ///
    /// Entering a disabled span and asking for its id are both no-ops, so
    /// nothing downstream has to know about the difference.
    fn span(self, kind: TaskKind, size: usize) -> Span {
        match self.0 {
            Some(it) => spawn_span(kind, size, it.loc, it.name),
            None => Span::none(),
        }
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
        Self(meta.span(TaskKind::Task, mem::size_of::<F>()))
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

/// A [`Waker`] that reports its operations to the console, on behalf of a task
/// that is not owned by the executor.
struct ShimWaker {
    inner: Waker,
    id: u64,
}

impl ShimWaker {
    const VTABLE: &'static RawWakerVTable = &RawWakerVTable::new(
        Self::clone_waker,
        Self::wake,
        Self::wake_by_ref,
        Self::drop_waker,
    );

    fn waker(inner: Waker, id: u64) -> Waker {
        let this = Arc::new(Self { inner, id });
        // The shim is a live waker of the task, and dropping it reports a
        // `waker.drop`, so report the matching `waker.clone` here.
        this.report(WakerOp::Clone);
        let raw = RawWaker::new(Arc::into_raw(this).cast(), Self::VTABLE);
        // SAFETY: the vtable operates on `Arc<ShimWaker>` pointers, which is
        // what we just leaked.
        unsafe { Waker::from_raw(raw) }
    }

    /// # Safety
    ///
    /// `ptr` must come from [`Arc::into_raw`] of a live `Arc<ShimWaker>`.
    unsafe fn borrow(ptr: *const ()) -> &'static Self {
        unsafe { &*ptr.cast::<Self>() }
    }

    /// # Safety
    ///
    /// `ptr` must come from [`Arc::into_raw`] of a live `Arc<ShimWaker>`, and
    /// the caller must give up its ownership of it.
    unsafe fn own(ptr: *const ()) -> Arc<Self> {
        unsafe { Arc::from_raw(ptr.cast::<Self>()) }
    }

    unsafe fn clone_waker(ptr: *const ()) -> RawWaker {
        unsafe { Self::borrow(ptr) }.report(WakerOp::Clone);
        unsafe { Arc::increment_strong_count(ptr.cast::<Self>()) };
        RawWaker::new(ptr, Self::VTABLE)
    }

    unsafe fn wake(ptr: *const ()) {
        let this = unsafe { Self::own(ptr) };
        this.report(WakerOp::Wake);
        this.inner.wake_by_ref();
    }

    unsafe fn wake_by_ref(ptr: *const ()) {
        let this = unsafe { Self::borrow(ptr) };
        this.report(WakerOp::WakeByRef);
        this.inner.wake_by_ref();
    }

    unsafe fn drop_waker(ptr: *const ()) {
        let this = unsafe { Self::own(ptr) };
        this.report(WakerOp::Drop);
    }

    #[inline]
    fn report(&self, op: WakerOp) {
        waker_op(self.id, op);
    }
}

/// A future instrumented as a `block_on` task, returned by
/// [`instrument_block_on`].
struct BlockOn<F> {
    span: Span,
    id: Option<u64>,
    /// The waker given to us by the runtime and the shim wrapping it, cached to
    /// avoid rebuilding the shim on every poll.
    waker: Option<(Waker, Waker)>,
    fut: F,
}

impl<F: Future> Future for BlockOn<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: we never move out of `fut`, and all other fields are `Unpin`.
        let this = unsafe { self.get_unchecked_mut() };
        let fut = unsafe { Pin::new_unchecked(&mut this.fut) };

        let _entered = this.span.enter();

        let Some(id) = this.id else {
            return fut.poll(cx);
        };

        if !matches!(&this.waker, Some((given, _)) if given.will_wake(cx.waker())) {
            let given = cx.waker().clone();
            let shim = ShimWaker::waker(given.clone(), id);
            this.waker = Some((given, shim));
        }

        let shim = &this.waker.as_ref().expect("waker was just set").1;
        fut.poll(&mut Context::from_waker(shim))
    }
}

/// Instrument a closure about to be handed to the blocking pool, so that it
/// shows up as a blocking task in the console.
///
/// The span is created here rather than on the pool thread, so that the task
/// shows up as soon as it is queued and the wait for a worker is reported as
/// idle time. It is entered around the closure itself, so that the time spent
/// in it is reported as busy time.
pub fn instrument_blocking<T, F: FnOnce() -> T>(meta: SpawnMeta, f: F) -> impl FnOnce() -> T {
    let span = meta.span(TaskKind::Blocking, mem::size_of::<F>());

    move || {
        let _entered = span.enter();
        f()
    }
}

/// Instrument a future blocked on by the runtime, so that it shows up as a
/// task in the console.
#[track_caller]
pub fn instrument_block_on<F: Future>(fut: F) -> impl Future<Output = F::Output> {
    let span = SpawnMeta::capture().span(TaskKind::BlockOn, mem::size_of::<F>());
    let id = span.id().map(|id| id.into_u64());

    BlockOn {
        span,
        id,
        waker: None,
        fut,
    }
}
