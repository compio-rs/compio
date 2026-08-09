//! Assert that the dispatcher names the tasks it runs, and points them at the
//! call they came from rather than at its own internals, for [`tokio-console`].
//!
//! [`tokio-console`]: https://github.com/tokio-rs/console
#![cfg(feature = "console")]

use std::{
    fmt::Debug,
    num::NonZeroUsize,
    sync::{Arc, Mutex},
};

use compio_dispatcher::Dispatcher;
use tracing::{
    Event, Metadata, Subscriber,
    field::{Field, Visit},
    span::{Attributes, Id, Record},
};

/// Where the console says a task came from.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Task {
    name: Option<String>,
    file: String,
    line: u64,
}

/// A subscriber recording the tasks `console-subscriber` would report.
///
/// `compio-executor` asserts what the spans hold; this only has to tell where
/// the dispatcher's own tasks point.
#[derive(Debug, Default, Clone)]
struct Recorder(Arc<Mutex<Vec<Task>>>);

impl Recorder {
    /// The tasks recorded under `name`, whichever thread spawned them.
    fn named(&self, name: &str) -> Vec<Task> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|it| it.name.as_deref() == Some(name))
            .cloned()
            .collect()
    }

    fn all(&self) -> Vec<Task> {
        self.0.lock().unwrap().clone()
    }
}

impl Subscriber for Recorder {
    fn enabled(&self, meta: &Metadata<'_>) -> bool {
        meta.name() == "runtime.spawn"
    }

    fn new_span(&self, attrs: &Attributes<'_>) -> Id {
        let mut task = Task::default();
        attrs.record(&mut TaskVisitor(&mut task));

        let mut tasks = self.0.lock().unwrap();
        tasks.push(task);
        Id::from_u64(tasks.len() as u64)
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, _event: &Event<'_>) {}

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}
}

struct TaskVisitor<'a>(&'a mut Task);

impl Visit for TaskVisitor<'_> {
    fn record_u64(&mut self, field: &Field, value: u64) {
        if field.name() == "loc.line" {
            self.0.line = value;
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "task.name" => self.0.name = Some(value.to_owned()),
            "loc.file" => self.0.file = value.to_owned(),
            _ => {}
        }
    }

    fn record_debug(&mut self, _field: &Field, _value: &dyn Debug) {}
}

const THREADS: usize = 2;

/// The workers spawn on threads of their own, so the recorder has to be the
/// global subscriber rather than this thread's. That it can only be installed
/// once is why this file holds a single test.
#[compio_macros::test]
async fn dispatched_tasks_are_named_and_point_at_their_caller() {
    let recorder = Recorder::default();
    tracing::subscriber::set_global_default(recorder.clone()).unwrap();

    let builder = Dispatcher::builder().worker_threads(NonZeroUsize::new(THREADS).unwrap());
    let built = u64::from(line!()) + 1;
    let dispatcher = builder.build().unwrap();

    let dispatched = u64::from(line!()) + 1;
    let handle = dispatcher.dispatch(|| std::future::ready(())).unwrap();
    handle.await.unwrap();
    dispatcher.join().await.unwrap();

    let tasks = recorder.named("dispatch");
    assert_eq!(
        tasks.len(),
        1,
        "one task per dispatch: {:?}",
        recorder.all()
    );
    assert_eq!(
        (tasks[0].file.as_str(), tasks[0].line),
        (file!(), dispatched),
        "a dispatched task points at the `dispatch` call, not at the worker thread that ran it"
    );

    let workers = recorder.named("dispatcher::worker");
    assert_eq!(
        workers.len(),
        THREADS,
        "one task per worker: {:?}",
        recorder.all()
    );
    for worker in &workers {
        assert_eq!(
            (worker.file.as_str(), worker.line),
            (file!(), built),
            "a worker points at the call that built the dispatcher, not into compio-dispatcher"
        );
    }
}
