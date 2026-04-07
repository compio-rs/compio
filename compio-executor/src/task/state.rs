use std::{
    fmt::{self, Debug},
    process::abort,
    sync::atomic::Ordering::{self, *},
};

use compio_log::trace;

/// Executor has scheduled the task to run, but it is not running.
const SCHEDULED: usize = 1 << 0;

/// A waker is scheduling from remote.
const SCHEDULING: usize = 1 << 1;

/// Not in the process of cloning and setting a waker to the task.
const NOT_SETTING_WAKER: usize = 1 << 2;

/// A waker is set to the task.
const HAS_WAKER: usize = 1 << 3;

/// The task is completed, and `FutureState` has switched to `result`.
const COMPLETED: usize = 1 << 4;

/// The result is present.
const HAS_RESULT: usize = 1 << 5;

/// The task is not cancelled. It might be cancelled because either the
/// `JoinHandle` is dropped without detaching or the Exector is dropped.
const NOT_CANCELLED: usize = 1 << 6;

const RC_SHIFT: usize = 7;
const RC_UNIT: usize = 1 << RC_SHIFT;
const RC_MAX: usize = usize::MAX >> RC_SHIFT;

#[repr(transparent)]
pub(crate) struct State(crate::AtomicUsize);

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
            .field("scheduling", &self.is_scheduling())
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
        const INIT: usize = NOT_SETTING_WAKER | NOT_CANCELLED;

        Self(crate::AtomicUsize::new((N * RC_UNIT) | INIT))
    }

    pub(crate) fn set_scheduled<const SET: bool>(&self) -> Snapshot {
        trace!(SET, "set_scheduled");

        if SET {
            Snapshot(self.0.fetch_or(SCHEDULED, AcqRel))
        } else {
            Snapshot(self.0.fetch_and(!SCHEDULED, AcqRel))
        }
    }

    pub(crate) fn set_scheduling<const SET: bool>(&self) -> Snapshot {
        trace!(SET, "set_scheduling");

        if SET {
            Snapshot(self.0.fetch_or(SCHEDULING, Release))
        } else {
            Snapshot(self.0.fetch_and(!SCHEDULING, Release))
        }
    }

    pub(crate) fn set_cancelled(&self) -> Snapshot {
        trace!("set_cancelled");

        Snapshot(self.0.fetch_and(!NOT_CANCELLED, AcqRel))
    }

    pub(crate) fn set_finished_running(&self) -> Snapshot {
        trace!("set_finished_running");
        Snapshot(self.0.fetch_or(COMPLETED | HAS_RESULT, AcqRel))
    }

    pub(crate) fn set_dropped(&self) -> Snapshot {
        trace!("set_dropped");

        Snapshot(self.0.fetch_and(!HAS_WAKER & !NOT_CANCELLED, AcqRel))
    }

    pub(crate) fn set_has_result<C: Consistency, const SET: bool>(&self) {
        trace!(SET, "set_has_result");
        if SET {
            self.0.fetch_or(HAS_RESULT, C::RELEASE);
        } else {
            self.0.fetch_and(!HAS_RESULT, C::RELEASE);
        }
    }

    pub(crate) fn setting_waker<const SET: bool>(&self) {
        trace!(SET, "setting_waker");
        if SET {
            self.0.fetch_and(!NOT_SETTING_WAKER, Release);
        } else {
            self.0.fetch_or(NOT_SETTING_WAKER, Release);
        }
    }

    pub(crate) fn set_has_waker<C: Consistency, const SET: bool>(&self) -> Snapshot {
        trace!(SET, "set_has_waker");
        if SET {
            Snapshot(self.0.fetch_or(HAS_WAKER | NOT_SETTING_WAKER, C::RELEASE))
        } else {
            Snapshot(self.0.fetch_and(!HAS_WAKER, C::RELEASE))
        }
    }

    /// Load the state with acquire ordering.
    pub(crate) fn load<C: Consistency>(&self) -> Snapshot {
        Snapshot(self.0.load(C::ACQUIRE))
    }

    pub(crate) fn inc(&self) -> Snapshot {
        let old = Snapshot(self.0.fetch_add(RC_UNIT, Release));
        trace!(?old, "inc");
        if old.count() == RC_MAX {
            abort()
        }
        old
    }

    /// Decrease the reference count by one and return the old state.
    pub(crate) fn dec(&self) -> Snapshot {
        let old = Snapshot(self.0.fetch_sub(RC_UNIT, AcqRel));
        trace!(?old, "dec");
        debug_assert!(old.count() >= 1, "Reference count underflow");
        old
    }
}

impl Snapshot {
    pub(crate) fn is_scheduled(&self) -> bool {
        self.0 & SCHEDULED != 0
    }

    pub(crate) fn is_scheduling(&self) -> bool {
        self.0 & SCHEDULING != 0
    }

    pub(crate) fn is_completed(&self) -> bool {
        self.0 & COMPLETED != 0
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.0 & NOT_CANCELLED == 0
    }

    pub(crate) fn is_setting_waker(&self) -> bool {
        self.0 & NOT_SETTING_WAKER == 0
    }

    pub(crate) fn has_waker(&self) -> bool {
        self.0 & HAS_WAKER != 0
    }

    pub(crate) fn has_result(&self) -> bool {
        self.0 & HAS_RESULT != 0
    }

    pub(crate) fn count(&self) -> usize {
        self.0 >> RC_SHIFT
    }
}
