//! Assert that the executor emits what [`tokio-console`] expects.
//!
//! [`tokio-console`]: https://github.com/tokio-rs/console
#![cfg(feature = "console")]

use std::{
    collections::{HashMap, HashSet},
    future::Future,
    pin::pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};

use compio_executor::{Executor, SpawnMeta, console};
use tracing::{
    Event, Metadata, Subscriber,
    field::{Field, Visit},
    span::{Attributes, Id, Record},
};

/// Everything we care about a `runtime.spawn` span.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Task {
    kind: String,
    id: u64,
    /// The console displays this in a column of its own, when present.
    name: Option<String>,
    size: u64,
    /// Label telling the executors of a thread-per-core application apart,
    /// which the console displays among the fields of the task.
    thread: String,
    file: String,
    line: u64,
    col: u64,
    /// Number of times the span was entered, which the console reports as the
    /// number of polls of the task.
    polls: usize,
    exits: usize,
    closed: bool,
    /// `op` values of the `runtime::waker` events of this task, in order.
    waker_ops: Vec<String>,
}

impl Task {
    fn wakes(&self) -> usize {
        self.count("waker.wake") + self.count("waker.wake_by_ref")
    }

    fn count(&self, op: &str) -> usize {
        self.waker_ops.iter().filter(|it| *it == op).count()
    }

    /// The console derives the number of live wakers of a task this way, and
    /// warns about tasks that have lost all of them.
    fn live_wakers(&self) -> isize {
        // `Waker::wake` consumes the waker without dropping it.
        let created = self.count("waker.clone") as isize;
        let destroyed = (self.count("waker.drop") + self.count("waker.wake")) as isize;
        created - destroyed
    }
}

/// A subscriber recording the same things `console-subscriber` would.
#[derive(Debug, Default, Clone)]
struct Recorder(Arc<Mutex<HashMap<u64, Task>>>);

impl Recorder {
    /// Install this recorder for the current thread.
    ///
    /// Only for the current one: an event reaches whichever subscriber is
    /// current where it is emitted, not the one its task's span holds, so a
    /// waker operation performed by a thread without a recorder goes
    /// unrecorded. No test wakes a task from another thread yet; one that does
    /// has to install the recorder there too, or watch its waker counts
    /// silently not balance.
    fn install(&self) -> tracing::subscriber::DefaultGuard {
        tracing::subscriber::set_default(self.clone())
    }

    /// The recorded tasks, in the order they were spawned.
    fn tasks(&self) -> Vec<Task> {
        let tasks = self.0.lock().unwrap();
        let mut tasks: Vec<_> = tasks.iter().collect();
        tasks.sort_unstable_by_key(|(span, _)| **span);
        tasks.into_iter().map(|(_, task)| task.clone()).collect()
    }

    /// The only recorded spawned task, ignoring the `block_on` one.
    fn spawned(&self) -> Task {
        let mut tasks = self.tasks();
        tasks.retain(|task| task.kind == "task");
        assert_eq!(tasks.len(), 1);
        tasks.into_iter().next().unwrap()
    }

    fn with(&self, id: &Id, f: impl FnOnce(&mut Task)) {
        if let Some(task) = self.0.lock().unwrap().get_mut(&id.into_u64()) {
            f(task);
        }
    }
}

impl Subscriber for Recorder {
    /// `console-subscriber` recognizes a task by this span name and a waker
    /// operation by this event target, whatever their other metadata, and
    /// ignores everything else. Filtering the same way keeps this recorder from
    /// tripping over the unrelated spans and events of the `enable_log`
    /// feature.
    fn enabled(&self, meta: &Metadata<'_>) -> bool {
        meta.name() == "runtime.spawn" || meta.target() == "runtime::waker"
    }

    fn new_span(&self, attrs: &Attributes<'_>) -> Id {
        assert!(
            attrs.parent().is_none() && attrs.is_root(),
            "task spans must not be nested, or the console attributes the polls of a task to its \
             parent"
        );

        let mut task = Task::default();
        attrs.record(&mut TaskVisitor(&mut task));

        // The id is only unique among live spans, which is enough here since
        // the recorder never forgets a task. Hold the lock across the whole
        // read-modify-write, or two threads pick the same id.
        let mut tasks = self.0.lock().unwrap();
        let id = Id::from_u64(tasks.len() as u64 + 1);
        tasks.insert(id.into_u64(), task);
        id
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let mut op = WakerVisitor::default();
        event.record(&mut op);
        let (id, op) = (op.id.expect("task.id"), op.op.expect("op"));
        self.with(&Id::from_u64(id), |task| task.waker_ops.push(op));
    }

    fn enter(&self, span: &Id) {
        self.with(span, |task| task.polls += 1);
    }

    fn exit(&self, span: &Id) {
        self.with(span, |task| task.exits += 1);
    }

    fn try_close(&self, span: Id) -> bool {
        self.with(&span, |task| task.closed = true);
        true
    }
}

struct TaskVisitor<'a>(&'a mut Task);

impl Visit for TaskVisitor<'_> {
    fn record_u64(&mut self, field: &Field, value: u64) {
        match field.name() {
            "task.id" => self.0.id = value,
            "size.bytes" => self.0.size = value,
            "loc.line" => self.0.line = value,
            "loc.col" => self.0.col = value,
            _ => {}
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "kind" => self.0.kind = value.to_owned(),
            "task.name" => self.0.name = Some(value.to_owned()),
            "thread" => self.0.thread = value.to_owned(),
            "loc.file" => self.0.file = value.to_owned(),
            _ => {}
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        untyped(field, value);
    }
}

#[derive(Default)]
struct WakerVisitor {
    op: Option<String>,
    id: Option<u64>,
}

impl Visit for WakerVisitor {
    fn record_u64(&mut self, field: &Field, value: u64) {
        if field.name() == "task.id" {
            self.id = Some(value);
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "op" {
            self.op = Some(value.to_owned());
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        untyped(field, value);
    }
}

/// Reject a field recorded without a type of its own.
///
/// `console-subscriber` keeps a field recorded that way as an opaque string, so
/// a field it is supposed to interpret must be recorded with its own type
/// instead. An `Option` that is `None` records nothing at all, rather than a
/// `None` for the console to display.
fn untyped(field: &Field, value: &dyn std::fmt::Debug) -> ! {
    panic!(
        "`{}` is recorded as `{value:?}` rather than with a type of its own",
        field.name()
    );
}

#[track_caller]
fn block_on<F: Future>(exe: &Executor, fut: F) -> F::Output {
    let fut = console::instrument_block_on(SpawnMeta::capture(), fut);
    let cx = &mut Context::from_waker(Waker::noop());
    let mut fut = pin!(fut);
    loop {
        if let Poll::Ready(res) = fut.as_mut().poll(cx) {
            return res;
        }
        exe.tick();
    }
}

async fn yield_now() {
    let mut yielded = false;
    std::future::poll_fn(move |cx| {
        if yielded {
            return Poll::Ready(());
        }
        yielded = true;
        cx.waker().wake_by_ref();
        Poll::Pending
    })
    .await
}

#[test]
fn spawned_task_is_reported() {
    let recorder = Recorder::default();
    let _guard = recorder.install();
    let exe = Executor::new();

    let line = line!() + 1;
    let handle = exe.spawn(async {
        yield_now().await;
        yield_now().await;
    });
    block_on(&exe, handle).unwrap();

    let task = recorder.spawned();
    assert_eq!(task.file, file!());
    assert_eq!(task.line, line as u64, "the caller of spawn is reported");
    assert_ne!(task.col, 0, "the column is recorded as a number");
    assert_ne!(task.size, 0);
    assert!(!task.thread.is_empty(), "the thread is labelled");

    // Initial poll plus one per yield.
    assert_eq!(task.polls, 3);
    assert_eq!(task.exits, task.polls);
    assert!(task.closed);

    assert_eq!(task.wakes(), 2, "one wake per yield");
    assert_eq!(
        task.live_wakers(),
        0,
        "wakers must balance out, or the console reports a lost waker: {:?}",
        task.waker_ops
    );
}

#[test]
fn blocked_on_future_is_reported_as_a_task() {
    let recorder = Recorder::default();
    let _guard = recorder.install();
    let exe = Executor::new();

    block_on(&exe, async {
        exe.spawn(yield_now()).await.unwrap();
    });

    let tasks = recorder.tasks();
    assert_eq!(tasks.len(), 2);
    assert_eq!(tasks[0].kind, "block_on");
    assert_eq!(tasks[0].file, file!());
    assert_eq!(tasks[1].kind, "task");
    assert!(tasks[0].polls >= 2 && tasks[1].polls >= 2);
    assert_eq!(tasks[0].live_wakers(), 0);
    assert_eq!(tasks[1].live_wakers(), 0);
}

#[test]
fn blocking_closure_is_reported_as_a_blocking_task() {
    let recorder = Recorder::default();
    let _guard = recorder.install();

    let line = line!() + 1;
    let f = console::instrument_blocking(SpawnMeta::capture(), {
        let recorder = recorder.clone();
        // The console counts one poll per entry of the span and measures the
        // busy time of the task as the time it stays entered, so the closure
        // must run while it is.
        move || recorder.tasks()[0].polls
    });

    let task = &recorder.tasks()[0];
    assert_eq!(
        task.kind, "blocking",
        "the console skips its future lints for this kind"
    );
    assert_eq!(task.file, file!());
    assert_eq!(task.line, line as u64, "the caller is reported");
    assert_eq!(task.polls, 0, "the task shows up as soon as it is queued");

    assert_eq!(f(), 1, "the closure runs while the span is entered");
    let task = &recorder.tasks()[0];
    assert_eq!(task.exits, 1, "and the span is left afterwards");
    assert!(task.waker_ops.is_empty(), "a closure has no waker");
}

#[test]
fn task_can_be_named() {
    let recorder = Recorder::default();
    let _guard = recorder.install();
    let exe = Executor::new();

    let handle = exe.spawn_at(yield_now(), SpawnMeta::capture().named("named"));
    block_on(&exe, handle).unwrap();

    assert_eq!(recorder.spawned().name.as_deref(), Some("named"));
}

#[test]
fn unnamed_task_has_no_name() {
    let recorder = Recorder::default();
    let _guard = recorder.install();
    let exe = Executor::new();

    block_on(&exe, exe.spawn(yield_now())).unwrap();

    assert_eq!(
        recorder.spawned().name,
        None,
        "the console leaves the column empty rather than showing a placeholder"
    );
}

#[test]
fn untracked_task_is_not_reported() {
    let recorder = Recorder::default();
    let _guard = recorder.install();
    let exe = Executor::new();

    let handle = exe.spawn_at(yield_now(), SpawnMeta::untracked());
    block_on(&exe, handle).unwrap();

    let tasks = recorder.tasks();
    assert_eq!(tasks.len(), 1, "only the `block_on` task: {tasks:?}");
    assert_eq!(tasks[0].kind, "block_on");
}

#[test]
fn untracked_blocked_on_future_is_not_reported() {
    let recorder = Recorder::default();
    let _guard = recorder.install();
    let exe = Executor::new();

    // The runtime blocks on futures of its own, which it leaves unreported.
    // Such a task has no id, so it is polled with the waker it was given
    // rather than a shim reporting the operations on it.
    let handle = exe.spawn(yield_now());
    let fut = console::instrument_block_on(SpawnMeta::untracked(), handle);
    let cx = &mut Context::from_waker(Waker::noop());
    let mut fut = pin!(fut);
    loop {
        if let Poll::Ready(res) = fut.as_mut().poll(cx) {
            res.unwrap();
            break;
        }
        exe.tick();
    }

    let tasks = recorder.tasks();
    assert_eq!(tasks.len(), 1, "only the spawned task: {tasks:?}");
    assert_eq!(tasks[0].kind, "task");
    assert_eq!(
        tasks[0].live_wakers(),
        0,
        "an unreported task wraps no waker of its own, so the ones of the task it polls still \
         balance out: {:?}",
        tasks[0].waker_ops
    );
}

#[test]
fn task_ids_are_unique_across_executors() {
    const EXECUTORS: usize = 4;

    let recorder = Recorder::default();
    // One executor per thread, as a thread-per-core application has.
    let threads: Vec<_> = (0..EXECUTORS)
        .map(|it| {
            let recorder = recorder.clone();
            std::thread::Builder::new()
                .name(format!("executor-{it}"))
                .spawn(move || {
                    // The subscriber is thread-local, so each thread installs
                    // the shared recorder for itself.
                    let _guard = recorder.install();
                    let exe = Executor::new();
                    block_on(&exe, exe.spawn(std::future::ready(()))).unwrap();
                })
                .expect("spawn a thread")
        })
        .collect();
    for thread in threads {
        thread.join().expect("the thread shouldn't panic");
    }

    let tasks = recorder.tasks();
    let ids: HashSet<_> = tasks.iter().map(|it| it.id).collect();
    assert_eq!(ids.len(), tasks.len(), "{tasks:?}");
    // The console displays this to tell the executors apart.
    let labels: HashSet<_> = tasks.iter().map(|it| it.thread.as_str()).collect();
    assert_eq!(labels.len(), EXECUTORS, "{labels:?}");
}

#[test]
fn dropped_task_closes_its_span() {
    let recorder = Recorder::default();
    let _guard = recorder.install();
    let exe = Executor::new();

    let handle = exe.spawn(std::future::pending::<()>());
    exe.tick();
    drop(handle);
    // Let the executor reclaim the cancelled task.
    exe.tick();
    exe.tick();

    let task = recorder.spawned();
    assert_eq!(task.polls, 1);
    assert!(task.closed, "the console would show it as running forever");
}

#[test]
fn panicking_task_closes_its_span() {
    let recorder = Recorder::default();
    let _guard = recorder.install();
    let exe = Executor::new();

    // The executor catches this, so the thread never unwinds and the task ends
    // like any other. See `task_span_closes_while_unwinding` for the other one.
    let handle = exe.spawn(async { panic!("task panic") });
    exe.tick();
    drop(handle);
    exe.tick();

    let task = recorder.spawned();
    assert_eq!(task.polls, 1);
    assert!(task.closed, "the console would show it as running forever");
}

#[test]
fn task_span_closes_while_unwinding() {
    let recorder = Recorder::default();
    let _guard = recorder.install();

    // Dropping a task while the thread unwinds deallocates it without running
    // the future's destructor, but the span is closed all the same.
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let exe = Executor::new();
        let _handle = exe.spawn(std::future::pending::<()>());
        exe.tick();
        panic!("unwind through a live task");
    }));

    assert!(panicked.is_err());
    let task = recorder.spawned();
    assert_eq!(task.polls, 1);
    assert!(task.closed, "the console would show it as running forever");
}
