use std::{
    io,
    pin::Pin,
    sync::atomic::{AtomicBool, Ordering},
    task::{Context, Poll, ready},
};

use futures_core::Stream;

use super::{Event, InternalEvent, sys::EventSource};

static ACTIVE_READER: AtomicBool = AtomicBool::new(false);

/// A local asynchronous stream of terminal events.
///
/// The stream must be created while a Compio runtime is active. It does not
/// enable raw mode or any optional terminal event modes. Use Crossterm's
/// terminal and event commands to configure those modes before polling.
///
/// Terminal input is a process-wide resource. Only one `EventStream` can be
/// active at a time. A second call to [`EventStream::new`] returns
/// [`io::ErrorKind::AlreadyExists`]. Dropping the stream cancels its pending
/// Compio operation and permits a new stream.
#[must_use = "streams do nothing unless polled"]
pub struct EventStream {
    source: EventSource,
    _lease: ReaderLease,
}

impl EventStream {
    /// Creates an event stream for the process terminal.
    ///
    /// # Errors
    ///
    /// Returns an error when there is no active Compio runtime, another event
    /// stream is active, or the process terminal cannot be opened.
    pub fn new() -> io::Result<Self> {
        let lease = ReaderLease::acquire()?;
        let source = EventSource::new()?;
        Ok(Self {
            source,
            _lease: lease,
        })
    }
}

impl Stream for EventStream {
    type Item = io::Result<Event>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            match ready!(Pin::new(&mut self.source).poll_next(cx)) {
                Some(Ok(InternalEvent::Event(event))) => {
                    return Poll::Ready(Some(Ok(event)));
                }
                Some(Ok(InternalEvent::Ignored)) => {}
                Some(Err(error)) => return Poll::Ready(Some(Err(error))),
                None => return Poll::Ready(None),
            }
        }
    }
}

struct ReaderLease;

impl ReaderLease {
    fn acquire() -> io::Result<Self> {
        ACTIVE_READER
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "a terminal event stream is already active",
                )
            })?;
        Ok(Self)
    }
}

impl Drop for ReaderLease {
    fn drop(&mut self) {
        ACTIVE_READER.store(false, Ordering::Release);
    }
}
