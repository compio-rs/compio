use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Wake, Waker},
};

use crate::runtime::send_wrapper::SendWrapper;

pub struct OptWaker {
    waker: Waker,
    current_thread: SendWrapper<()>,
    is_woke: AtomicBool,
}

impl OptWaker {
    pub(crate) fn new(waker: Waker) -> Arc<Self> {
        Arc::new(Self {
            waker,
            current_thread: SendWrapper::new(()),
            is_woke: AtomicBool::new(false),
        })
    }

    pub fn is_woke(&self) -> bool {
        self.is_woke.load(Ordering::Acquire)
    }
}

impl Wake for OptWaker {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        if !self.current_thread.valid() {
            self.waker.wake_by_ref();
        }
        self.is_woke.store(true, Ordering::Release);
    }
}
