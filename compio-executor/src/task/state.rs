use std::{
    fmt::{self, Debug},
    process::abort,
    sync::atomic::{
        AtomicUsize,
        Ordering::{self, *},
    },
};

use compio_log::trace;

/// Executor has scheduled the task to run, but it is not running.
const SHCEDULED: usize = 1 << 0;

/// In the process of cloning and setting a waker to the task.
const SETTING_WAKER: usize = 1 << 1;

/// A waker is set to the task.
const HAS_WAKER: usize = 1 << 2;

/// The task is completed, and `FutureState` has switched to `result`.
const COMPLETED: usize = 1 << 3;

/// The result is present.
const HAS_RESULT: usize = 1 << 4;

/// The task is cancelled because either the `JoinHandle` is dropped without
/// detaching or the Exector is dropped.
const CANCELLED: usize = 1 << 5;

const RC_SHIFT: usize = 6;
const RC_UNIT: usize = 1 << RC_SHIFT;
const RC_MASK: usize = !(RC_UNIT - 1);

#[repr(transparent)]
pub(crate) struct State(AtomicUsize);

#[derive(Clone, Copy)]
#[repr(transparent)]
pub(crate) struct Snapshot(usize);

impl Debug for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.load::<Weak>().fmt(f)
    }
}

impl Debug for Snapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("State")
            .field("count", &self.count())
            .field("scheduled", &self.is_scheduled())
            .field("setting_waker", &self.is_setting_waker())
            .field("has_waker", &self.has_waker())
            .field("completed", &self.is_completed())
            .field("has_result", &self.has_result())
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

pub(super) trait Consistency {
    const ACQUIRE: Ordering;
    const RELEASE: Ordering;
}

pub(super) struct Strong;
pub(super) struct Weak;

impl Consistency for Strong {
    const ACQUIRE: Ordering = Acquire;
    const RELEASE: Ordering = Release;
}

impl Consistency for Weak {
    const ACQUIRE: Ordering = Relaxed;
    const RELEASE: Ordering = Relaxed;
}

impl State {
    /// Create a state with two given count.
    pub(crate) fn new<const N: usize>() -> Self {
        Self(AtomicUsize::new(N * RC_UNIT))
    }

    pub(crate) fn set_scheduled<const SET: bool>(&self) {
        trace!(SET, "set_scheduled");
        // it's fine to set scheduled flag with relaxed ordering since the executor will
        // deduplicate `TaskId`s. It will not cause any correctness issue if the flag is
        // not correctly observed. The only issue may be duplicate task id will jammed
        // the sync queue, causing threads to yield.
        if SET {
            self.0.fetch_or(SHCEDULED, Relaxed);
        } else {
            self.0.fetch_and(!SHCEDULED, Relaxed);
        }
    }

    pub(crate) fn set_cancelled(&self) -> Snapshot {
        trace!("set_cancelled");

        Snapshot(self.0.fetch_or(CANCELLED, Release))
    }

    pub(crate) fn set_finished_running(&self) -> Snapshot {
        trace!("set_finished_running");
        Snapshot(self.0.fetch_or(COMPLETED | HAS_RESULT, Release))
    }

    pub(crate) fn set_dropped(&self) -> Snapshot {
        trace!("set_dropped");
        // Only clear HAS_WAKER, not HAS_RESULT
        // The result will be dropped either by JoinHandle taking it, or by Task's Drop
        const DROP_MASK: usize = !HAS_WAKER;

        let old = Snapshot(self.0.fetch_and(DROP_MASK, Release));

        if !old.is_cancelled() {
            Snapshot(self.0.fetch_or(CANCELLED, Release))
        } else {
            old
        }
    }

    pub(crate) fn set_has_result<C: Consistency, const SET: bool>(&self) {
        trace!(SET, "set_has_result");
        if SET {
            self.0.fetch_or(HAS_RESULT, C::RELEASE);
        } else {
            self.0.fetch_and(!HAS_RESULT, C::RELEASE);
        }
    }

    pub(crate) fn seting_waker<const SET: bool>(&self) {
        trace!(SET, "setting_waker");
        if SET {
            self.0.fetch_or(SETTING_WAKER, Release);
        } else {
            self.0.fetch_and(!SETTING_WAKER, Release);
        }
    }

    pub(crate) fn set_has_waker<C: Consistency, const SET: bool>(&self) {
        trace!(SET, "set_has_waker");
        if SET {
            // Clear SETTING_WAKER and set HAS_WAKER atomically
            // We need to do this in two steps since we can't OR and AND at the same time
            self.0.fetch_and(!SETTING_WAKER, C::RELEASE);
            self.0.fetch_or(HAS_WAKER, C::RELEASE);
        } else {
            self.0.fetch_and(!HAS_WAKER, C::RELEASE);
        }
    }

    /// Load the state with acquire ordering.
    pub(crate) fn load<C: Consistency>(&self) -> Snapshot {
        Snapshot(self.0.load(C::ACQUIRE))
    }

    pub(crate) fn inc(&self) -> Snapshot {
        let old = self.0.fetch_add(RC_UNIT, Release);
        trace!(old_count = Snapshot(old).count(), "inc");
        if old >= RC_MASK {
            abort()
        }
        Snapshot(old)
    }

    /// Decrease the reference count by one and return the old state.
    pub(crate) fn dec(&self) -> Snapshot {
        let old = self.0.fetch_sub(RC_UNIT, AcqRel);
        trace!(old_count = Snapshot(old).count(), "dec");
        debug_assert!(old >= RC_UNIT, "Reference count underflow");
        Snapshot(old)
    }
}

impl Snapshot {
    pub(crate) fn is_scheduled(&self) -> bool {
        self.0 & SHCEDULED != 0
    }

    pub(crate) fn is_completed(&self) -> bool {
        self.0 & COMPLETED != 0
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.0 & CANCELLED != 0
    }

    pub(crate) fn is_setting_waker(&self) -> bool {
        self.0 & SETTING_WAKER != 0
    }

    pub(crate) fn has_waker(&self) -> bool {
        self.0 & HAS_WAKER != 0
    }

    pub(crate) fn has_result(&self) -> bool {
        self.0 & HAS_RESULT != 0
    }

    pub(crate) fn count(&self) -> usize {
        (self.0 & RC_MASK) >> RC_SHIFT
    }
}
