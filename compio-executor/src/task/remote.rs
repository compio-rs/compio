use std::{
    marker::PhantomData,
    mem::MaybeUninit,
    ptr::NonNull,
    task::{Context, Poll},
    thread,
};

use compio_log::{instrument, trace};

use crate::{
    PanicResult,
    task::{
        Header,
        state::{Snapshot, Strong},
    },
    util::abort_on_panic,
};

/// A remote view into the task allocation, which is used when the [`Task`]
/// is accessed from a different thread than it's created on.
#[repr(transparent)]
pub(super) struct Remote<'a> {
    ptr: NonNull<Header>,
    marker: PhantomData<&'a Header>,
}

impl<'a> Remote<'a> {
    pub fn new(ptr: NonNull<Header>) -> Self {
        Self {
            ptr,
            marker: PhantomData,
        }
    }

    pub fn schedule(&self) {
        let state = self.header().state.set_scheduled::<true>();
        if state.is_scheduled()
            || state.is_scheduling()
            || state.is_completed()
            || state.is_cancelled()
        {
            return;
        }
        self.header().state.set_scheduling::<true>();

        // Check if shared pointer is still valid
        let Some(shared) = self.header().shared.with(|ptr| unsafe { (*ptr).as_ref() }) else {
            self.header().state.set_scheduling::<false>();
            return;
        };

        let mut notified = false;
        while shared.sync.push(self.header().id).is_err() {
            if !notified && let Some(ref waker) = shared.waker {
                abort_on_panic(|| waker.wake_by_ref());
                notified = true;
            } else if self.header().state.load::<Strong>().is_cancelled() {
                return;
            } else {
                thread::yield_now()
            }
        }
        if !notified && let Some(ref waker) = shared.waker {
            abort_on_panic(|| waker.wake_by_ref());
        }

        self.header().state.set_scheduling::<false>();
    }

    pub unsafe fn poll<T>(&self, cx: &mut Context<'_>) -> Poll<Option<PanicResult<T>>> {
        instrument!(compio_log::Level::TRACE, "Remote::poll", id = ?self.header().id);
        let state = self.state();

        trace!(?state);

        debug_assert!(
            !state.is_completed() || state.has_result() || state.is_cancelled(),
            "Should not poll after the result is taken"
        );

        // Check if the task has completed with a result first
        if state.is_completed() && state.has_result() {
            self.header().state.set_has_result::<Strong, false>();

            let mut res = MaybeUninit::<PanicResult<T>>::uninit();
            let target = NonNull::from_mut(&mut res).cast();
            unsafe { (self.header().vtable.take_result)(self.ptr, target) };

            return Poll::Ready(Some(unsafe { res.assume_init() }));
        }

        // Check cancellation - cancelled tasks without result return None
        if state.is_cancelled() {
            return Poll::Ready(None);
        }

        // Task is not completed yet, set up waker
        if !state.is_completed() {
            return self.header().waker.with_mut(|waker| {
                let waker = unsafe { &mut *waker };

                if state.has_waker() {
                    if cx.waker().will_wake(unsafe { waker.assume_init_ref() }) {
                        return Poll::Pending;
                    }

                    self.header().state.setting_waker::<true>();

                    unsafe { MaybeUninit::assume_init_drop(waker) };
                } else {
                    self.header().state.setting_waker::<true>();
                }

                waker.write(abort_on_panic(|| cx.waker().clone()));

                self.header().state.set_has_waker::<Strong, true>();

                Poll::Pending
            });
        }

        // Task is completed but has no result (shouldn't happen)
        Poll::Ready(None)
    }

    fn header(&self) -> &Header {
        unsafe { self.ptr.as_ref() }
    }

    fn state(&self) -> Snapshot {
        self.header().state.load::<Strong>()
    }
}
