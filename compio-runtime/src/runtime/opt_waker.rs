use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Wake, Waker},
};

#[cfg(not(feature = "notify-always"))]
use crate::runtime::send_wrapper::SendWrapper;

/// An optimized waker that avoids unnecessary wake-ups on the same thread.
pub struct OptWaker {
    waker: Waker,
    #[cfg(not(feature = "notify-always"))]
    current_thread: SendWrapper<()>,
    is_woke: AtomicBool,
}

impl OptWaker {
    pub(crate) fn new(waker: Waker) -> Arc<Self> {
        Arc::new(Self {
            waker,
            #[cfg(not(feature = "notify-always"))]
            current_thread: SendWrapper::new(()),
            is_woke: AtomicBool::new(false),
        })
    }

    /// Returns `true` if the waker has been woke, and resets the state to not
    /// woke.
    pub fn reset(&self) -> bool {
        self.is_woke.swap(false, Ordering::AcqRel)
    }

    #[inline]
    fn should_wake(&self) -> bool {
        #[cfg(feature = "notify-always")]
        {
            true
        }
        #[cfg(not(feature = "notify-always"))]
        {
            !self.current_thread.valid()
        }
    }
}

impl Wake for OptWaker {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        if !self.is_woke.swap(true, Ordering::AcqRel) && self.should_wake() {
            self.waker.wake_by_ref();
        }
    }
}
