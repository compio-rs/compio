use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Wake, Waker},
};

/// An optimized waker that avoids unnecessary wake-ups on the same thread.
pub struct OptWaker {
    waker: Waker,
    is_woke: AtomicBool,
}

impl OptWaker {
    pub(crate) fn new(waker: Waker) -> Arc<Self> {
        Arc::new(Self {
            waker,
            is_woke: AtomicBool::new(false),
        })
    }

    /// Returns `true` if the waker has been woke, and resets the state to not
    /// woke.
    pub fn reset(&self) -> bool {
        self.is_woke.swap(false, Ordering::AcqRel)
    }
}

impl Wake for OptWaker {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        if !self.is_woke.swap(true, Ordering::AcqRel) {
            self.waker.wake_by_ref();
        }
    }
}
