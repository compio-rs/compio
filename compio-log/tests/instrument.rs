#![cfg(not(feature = "enable_log"))]

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use compio_log::Instrument;
use tracing::{Subscriber, span::Id};
use tracing_subscriber::{
    Layer,
    layer::{Context, SubscriberExt},
    registry::Registry,
};

struct Closed(Arc<AtomicBool>);

impl<S: Subscriber> Layer<S> for Closed {
    fn on_close(&self, _id: Id, _ctx: Context<'_, S>) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[test]
fn in_current_span_does_not_keep_span_alive() {
    let closed = Arc::new(AtomicBool::new(false));
    let subscriber = Registry::default().with(Closed(closed.clone()));

    tracing::subscriber::with_default(subscriber, || {
        let span = tracing::info_span!("startup");
        let guard = span.enter();
        // Outlives the span, like a spawned worker would.
        let worker = std::future::pending::<()>().in_current_span();
        drop(guard);
        drop(span);

        assert!(closed.load(Ordering::SeqCst));

        drop(worker);
    });
}
