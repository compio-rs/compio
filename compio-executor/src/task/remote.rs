use std::{
    marker::PhantomData,
    mem::MaybeUninit,
    ops::ControlFlow,
    ptr::NonNull,
    sync::atomic::Ordering,
    task::{Context, Poll},
};

use compio_log::{instrument, trace};

use crate::{
    PanicResult,
    task::{
        Header,
        state::{Snapshot, Strong},
    },
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
        instrument!(compio_log::Level::TRACE, "Remote::schedule", id = ?self.header().id);

        let state = self.header().state.start_scheduling();

        trace!(?state);

        if state.is_scheduled()
            || state.is_scheduling()
            || state.is_completed()
            || state.is_cancelled()
        {
            self.header().state.finish_scheduling();
            return;
        }

        // Load shared pointer - it should always be valid since we keep it until
        // Executor drops
        let Some(shared) = (unsafe { self.header().shared.load(Ordering::Acquire).as_ref() })
        else {
            self.header().state.finish_scheduling();
            return;
        };

        crate::panic_guard!();

        let mut notified = false;
        while shared.sync.push(self.header().id).is_err() {
            if !notified && let Some(ref waker) = shared.waker {
                waker.wake_by_ref();
                notified = true;
            } else if self.header().state.load::<Strong>().is_cancelled() {
                self.header().state.finish_scheduling();
                return;
            } else {
                crate::yield_now()
            }
        }
        if !notified && let Some(ref waker) = shared.waker {
            waker.wake_by_ref();
        }

        self.header().state.finish_scheduling();
    }

    pub unsafe fn poll<T>(&self, cx: &mut Context<'_>) -> Poll<Option<PanicResult<T>>> {
        instrument!(compio_log::Level::TRACE, "Remote::poll", id = ?self.header().id);
        let mut state = self.state();

        loop {
            trace!(?state);

            debug_assert!(state.has_result() || !state.is_completed() || state.is_cancelled());

            // The task is completed, take the result
            if state.has_result() {
                self.header().state.set_has_result::<Strong, false>();

                let mut res = MaybeUninit::<PanicResult<T>>::uninit();
                let target = NonNull::from_mut(&mut res).cast();
                unsafe { (self.header().vtable.take_result)(self.ptr, target) };

                break Poll::Ready(Some(unsafe { res.assume_init() }));
            }

            // The task is cancelled without result, return None
            if state.is_cancelled() {
                break Poll::Ready(None);
            }

            // Task is not completed yet, set up waker
            if !state.is_completed() {
                if state.has_waker()
                    && let Some(poll) = self.header().waker.with(|waker| {
                        cx.waker()
                            .will_wake(unsafe { (&*waker).assume_init_ref() })
                            .then_some(Poll::Pending)
                    })
                {
                    break poll;
                };

                let res = self.header().waker.with_mut(|ptr| {
                    crate::panic_guard!();

                    state = self.header().state.start_setting_waker();

                    // The task was cancelled after last check
                    if state.is_cancelled() {
                        self.header().state.finish_setting_waker::<false>();

                        return ControlFlow::Break(Poll::Ready(None));
                    }

                    // SAFETY: We're in SETTING_WAKER state, Executor will not access the waker
                    // until we're finished.
                    let waker = unsafe { &mut *ptr };

                    if state.has_waker() {
                        unsafe { waker.assume_init_drop() };
                    }

                    // Executor finished running after last check
                    if state.is_completed() && state.has_result() {
                        // It's waiting for us to stop. Finish setting waker here.
                        self.header().state.finish_setting_waker::<false>();

                        return ControlFlow::Continue(state);
                    }

                    // We're in the critical section, executor will wait for us to finish
                    waker.write(cx.waker().clone());

                    let state = self.header().state.finish_setting_waker::<true>();

                    // Executor dropped the task during setting waker
                    if state.is_cancelled() {
                        // Executor has cancelled the task - there will be no more access to the
                        // waker, so it's fine to just drop it here.
                        unsafe { waker.assume_init_drop() };

                        self.header().state.set_has_waker::<Strong, false>();

                        ControlFlow::Break(Poll::Ready(None))
                    } else {
                        ControlFlow::Break(Poll::Pending)
                    }
                });
                match res {
                    // The task was completed during setting waker, loop again so that we can
                    // retrieve the result
                    ControlFlow::Continue(s) => state = s,
                    ControlFlow::Break(poll) => break poll,
                }
            }
        }
    }

    fn header(&self) -> &Header {
        unsafe { self.ptr.as_ref() }
    }

    fn state(&self) -> Snapshot {
        self.header().state.load::<Strong>()
    }
}
