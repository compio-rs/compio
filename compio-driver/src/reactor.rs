//! A pluggable readiness backend for the polling driver.

use std::{
    fmt, io,
    num::NonZeroUsize,
    os::fd::{AsRawFd, BorrowedFd, RawFd},
    sync::Arc,
    time::Duration,
};

pub use polling::Event;
use polling::{Events, Poller};

/// The readiness backend of the polling driver.
///
/// It follows the contract of [`polling::Poller`]: interests are one-shot, so
/// an fd that has been reported is not reported again until it is
/// [`modify`](Reactor::modify)-ed. The driver owns its reactor and calls it
/// from the thread that drives it; another thread wakes a blocked
/// [`wait`](Reactor::wait) through the reactor's [`Notifier`]. The OS poller is
/// the default; install another with
/// [`ProactorBuilder::reactor`](crate::ProactorBuilder::reactor).
pub trait Reactor: AsRawFd + Send {
    /// Start watching `fd` for the interests in `event`.
    ///
    /// # Safety
    ///
    /// `fd` must be [`delete`](Reactor::delete)-d before it is closed.
    unsafe fn add(&self, fd: RawFd, event: Event) -> io::Result<()>;

    /// Replace the interests watched on `fd`.
    fn modify(&self, fd: BorrowedFd<'_>, event: Event) -> io::Result<()>;

    /// Stop watching `fd`.
    fn delete(&self, fd: BorrowedFd<'_>) -> io::Result<()>;

    /// Block until a watched fd is ready, `timeout` passes, or the
    /// [`Notifier`] is notified, and append the ready events to `events`.
    fn wait(&mut self, events: &mut Vec<Event>, timeout: Option<Duration>) -> io::Result<()>;

    /// A handle that wakes this reactor's [`wait`](Reactor::wait) from any
    /// thread.
    fn notifier(&self) -> Box<dyn Notifier>;
}

/// Wakes a [`Reactor`] blocked in [`wait`](Reactor::wait).
pub trait Notifier: Send + Sync {
    /// End the current or next [`wait`](Reactor::wait) of the reactor this
    /// came from.
    fn notify(&self) -> io::Result<()>;
}

/// The OS poller as a [`Reactor`]: the polling driver's default backend, and a
/// base for a reactor that adds behaviour around it.
pub struct OsReactor {
    poller: Arc<Poller>,
    events: Events,
}

impl OsReactor {
    /// Create an OS poller reporting up to `capacity` events per wait, or the
    /// `polling` crate's default when `capacity` is zero.
    pub fn new(capacity: u32) -> io::Result<Self> {
        let events = match NonZeroUsize::new(capacity as _) {
            Some(cap) => Events::with_capacity(cap),
            None => Events::new(),
        };
        Ok(Self {
            poller: Arc::new(Poller::new()?),
            events,
        })
    }
}

impl AsRawFd for OsReactor {
    fn as_raw_fd(&self) -> RawFd {
        self.poller.as_raw_fd()
    }
}

impl Reactor for OsReactor {
    unsafe fn add(&self, fd: RawFd, event: Event) -> io::Result<()> {
        unsafe { self.poller.add(fd, event) }
    }

    fn modify(&self, fd: BorrowedFd<'_>, event: Event) -> io::Result<()> {
        self.poller.modify(fd, event)
    }

    fn delete(&self, fd: BorrowedFd<'_>) -> io::Result<()> {
        self.poller.delete(fd)
    }

    fn wait(&mut self, events: &mut Vec<Event>, timeout: Option<Duration>) -> io::Result<()> {
        self.events.clear();
        self.poller.wait(&mut self.events, timeout)?;
        events.extend(self.events.iter());
        Ok(())
    }

    fn notifier(&self) -> Box<dyn Notifier> {
        Box::new(OsNotifier(self.poller.clone()))
    }
}

struct OsNotifier(Arc<Poller>);

impl Notifier for OsNotifier {
    fn notify(&self) -> io::Result<()> {
        self.0.notify()
    }
}

/// Creates the [`Reactor`] for each proactor a builder builds, on the thread
/// that will drive it.
/// Separate factory type because closure cannot implement Debug
#[derive(Clone)]
pub(crate) struct ReactorFactory(
    pub(crate) Arc<dyn Fn() -> io::Result<Box<dyn Reactor>> + Send + Sync>,
);

impl fmt::Debug for ReactorFactory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ReactorFactory")
    }
}
