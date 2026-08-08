//! Assert that the compatibility layer reports the future it executes to
//! [`tokio-console`].
//!
//! [`tokio-console`]: https://github.com/tokio-rs/console
#![cfg(feature = "console")]

use std::{
    fmt::Debug,
    sync::{Arc, Mutex},
};

use compio_compat::{Adapter, RuntimeCompat};
use compio_runtime::Runtime;
use tracing::{
    Event, Metadata, Subscriber,
    field::{Field, Visit},
    span::{Attributes, Id, Record},
};

/// What the console tells the tasks of a runtime apart by.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Task {
    kind: String,
    /// Displayed in a column of its own, when present.
    name: Option<String>,
    /// Number of times the span was entered, which the console reports as the
    /// number of polls of the task.
    polls: usize,
    exits: usize,
    /// Where the console tells the user to look for the task.
    file: String,
    line: u64,
}

/// A subscriber recording the tasks `console-subscriber` would report.
///
/// `compio-executor` asserts what the spans hold; this only has to tell which
/// of them the compatibility layer creates.
#[derive(Debug, Default, Clone)]
struct Recorder(Arc<Mutex<State>>);

#[derive(Debug, Default)]
struct State {
    tasks: Vec<Task>,
    /// Task spans currently entered. The console reads a span held while
    /// another task is polled as the two being nested, and charges the inner
    /// one's time to the outer.
    entered: usize,
    deepest: usize,
}

impl Recorder {
    fn tasks(&self) -> Vec<Task> {
        self.0.lock().unwrap().tasks.clone()
    }

    fn deepest(&self) -> usize {
        self.0.lock().unwrap().deepest
    }
}

impl Subscriber for Recorder {
    fn enabled(&self, meta: &Metadata<'_>) -> bool {
        meta.name() == "runtime.spawn"
    }

    fn new_span(&self, attrs: &Attributes<'_>) -> Id {
        let mut task = Task::default();
        attrs.record(&mut TaskVisitor(&mut task));

        let state = &mut *self.0.lock().unwrap();
        state.tasks.push(task);
        Id::from_u64(state.tasks.len() as u64)
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, _event: &Event<'_>) {}

    fn enter(&self, span: &Id) {
        let state = &mut *self.0.lock().unwrap();
        state.tasks[span.into_u64() as usize - 1].polls += 1;
        state.entered += 1;
        state.deepest = state.deepest.max(state.entered);
    }

    fn exit(&self, span: &Id) {
        let state = &mut *self.0.lock().unwrap();
        state.tasks[span.into_u64() as usize - 1].exits += 1;
        state.entered -= 1;
    }
}

struct TaskVisitor<'a>(&'a mut Task);

impl Visit for TaskVisitor<'_> {
    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "kind" => self.0.kind = value.to_owned(),
            "task.name" => self.0.name = Some(value.to_owned()),
            "loc.file" => self.0.file = value.to_owned(),
            _ => {}
        }
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        if field.name() == "loc.line" {
            self.0.line = value;
        }
    }

    fn record_debug(&mut self, _field: &Field, _value: &dyn Debug) {}
}

async fn test_impl<A: Adapter>() {
    let recorder = Recorder::default();
    let _guard = tracing::subscriber::set_default(recorder.clone());

    let runtime = RuntimeCompat::<A>::new(Runtime::new().unwrap()).unwrap();
    let f = async {
        compio_runtime::spawn(std::future::ready(())).await.unwrap();
        42
    };
    let called = u64::from(line!()) + 1;
    let answer = runtime.execute(f).await;
    assert_eq!(answer, 42);

    let tasks = recorder.tasks();
    let executed: Vec<_> = tasks
        .iter()
        .filter(|it| it.name.as_deref() == Some("execute"))
        .collect();
    assert_eq!(executed.len(), 1, "one task per executed future: {tasks:?}");
    assert_eq!(
        executed[0].kind, "block_on",
        "the console has no kind of its own for a future executed this way, so the name is what \
         tells it apart from one the runtime blocks on"
    );
    assert!(
        tasks.iter().any(|it| it.kind == "task"),
        "the tasks it spawns are reported alongside it: {tasks:?}"
    );
    assert!(
        executed[0].polls >= 2,
        "the future does not finish in one poll: {tasks:?}"
    );
    assert_eq!(
        executed[0].exits, executed[0].polls,
        "its span is left between polls rather than held across the `.await`s of `execute`, which \
         would report the time the task is idle as time spent polling it: {tasks:?}"
    );
    assert_eq!(
        recorder.deepest(),
        1,
        "and left before the runtime polls the tasks it spawned, or the console would read them \
         as nested in it and charge their time to it as well: {tasks:?}"
    );
    assert_eq!(
        (executed[0].file.as_str(), executed[0].line),
        (file!(), called),
        "it points at the call to `execute` rather than into the compatibility layer: {tasks:?}"
    );
}

#[cfg(feature = "tokio")]
#[tokio::test]
async fn tokio() {
    test_impl::<compio_compat::TokioAdapter>().await;
}

#[cfg(feature = "futures")]
#[test]
fn futures() {
    futures_executor::block_on(async {
        test_impl::<compio_compat::FuturesAdapter>().await;
    })
}
