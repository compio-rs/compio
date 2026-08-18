//! Assert that the tasks compio-quic spawns are named, and that the one
//! driving a connection points at the call that opened it, for
//! [`tokio-console`].
//!
//! [`tokio-console`]: https://github.com/tokio-rs/console
#![cfg(feature = "console")]

use std::{
    fmt::Debug,
    sync::{Arc, Mutex},
};

use compio_quic::{ClientBuilder, Endpoint};
use futures_util::join;
use tracing::{
    Event, Metadata, Subscriber,
    field::{Field, Visit},
    span::{Attributes, Id, Record},
};

#[allow(dead_code)]
mod common;
use common::config_pair;

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
/// the tasks of an endpoint point.
#[derive(Debug, Default, Clone)]
struct Recorder(Arc<Mutex<Vec<Task>>>);

impl Recorder {
    /// The tasks recorded under `name`, in the order they were spawned.
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

/// One endpoint serving both ends, so that a connection opened here and one
/// accepted here are told apart by their location alone.
#[compio_macros::test]
async fn connections_point_at_the_call_that_opened_them() {
    let recorder = Recorder::default();
    let _guard = tracing::subscriber::set_default(recorder.clone());

    let (server_config, client_config) = config_pair(None);
    let created = u64::from(line!()) + 1;
    let mut endpoint = Endpoint::server("127.0.0.1:0", server_config)
        .await
        .unwrap();
    endpoint.default_client_config = Some(client_config);
    let addr = endpoint.local_addr().unwrap();

    let connected = u64::from(line!()) + 2;
    let (client, server) = join!(
        async { endpoint.connect(addr, "localhost", None).unwrap().await },
        async {
            let incoming = endpoint.wait_incoming().await.unwrap();
            incoming.accept().unwrap().await
        },
    );
    let accepted = connected + 3;
    client.unwrap();
    server.unwrap();

    let mut lines: Vec<_> = recorder
        .named("quic::connection")
        .iter()
        .map(|it| (it.file.clone(), it.line))
        .collect();
    lines.sort();
    assert_eq!(
        lines,
        [
            (file!().to_owned(), connected),
            (file!().to_owned(), accepted)
        ],
        "a connection points at the `connect` or `accept` that opened it, not into compio-quic: \
         {:?}",
        recorder.all()
    );

    let workers = recorder.named("quic::endpoint");
    assert_eq!(workers.len(), 1, "one worker: {:?}", recorder.all());
    assert_eq!(
        (workers[0].file.as_str(), workers[0].line),
        (file!(), created),
        "the worker points at the call that created the endpoint: {:?}",
        workers[0]
    );
}

/// The builders reach an endpoint through `Endpoint::client`, so their `bind`
/// is the longest chain the location has to survive.
#[compio_macros::test]
async fn a_built_endpoint_points_at_the_bind_that_created_it() {
    let recorder = Recorder::default();
    let _guard = tracing::subscriber::set_default(recorder.clone());

    let builder = ClientBuilder::new_with_no_server_verification();
    let bound = u64::from(line!()) + 1;
    let endpoint = builder.bind("127.0.0.1:0").await.unwrap();
    endpoint.shutdown().await.unwrap();

    let workers = recorder.named("quic::endpoint");
    assert_eq!(workers.len(), 1, "one worker: {:?}", recorder.all());
    assert_eq!(
        (workers[0].file.as_str(), workers[0].line),
        (file!(), bound),
        "the worker points at the `bind` that created the endpoint: {:?}",
        workers[0]
    );
}
