use std::{
    marker::PhantomData,
    mem::MaybeUninit,
    ptr::NonNull,
    task::{Context, Poll},
};

use compio_log::{instrument, trace};

use crate::{
    PanicResult,
    task::{
        Header,
        state::{Snapshot, Strong, Weak},
    },
    util::abort_on_panic,
};

/// A local view into the task allocation, which enables some optimizations.
#[repr(transparent)]
pub(super) struct Local<'a> {
    ptr: NonNull<Header>,
    marker: PhantomData<&'a Header>,
}

impl<'a> Local<'a> {
    /// # Safety
    ///
    /// The caller must guarantee that this is created on the same thread as the
    /// task allocation.
    #[inline(always)]
    pub unsafe fn new(ptr: NonNull<Header>) -> Self {
        Self {
            ptr,
            marker: PhantomData,
        }
    }

    pub fn schedule(&self) {
        let Some(shared) = self.header().shared.with(|ptr| unsafe { (*ptr).as_ref() }) else {
            return;
        };

        // SAFETY: Type invariant
        let queue = unsafe { shared.queue.get_unchecked() };
        while let Some(id) = shared.sync.pop() {
            queue.make_hot(id);
        }
        queue.make_hot(self.header().id);

        if cfg!(feature = "notify-always")
            && let Some(ref waker) = shared.waker
        {
            waker.wake_by_ref()
        }
    }

    pub unsafe fn poll<T>(&self, cx: &mut Context<'_>) -> Poll<Option<PanicResult<T>>> {
        let state = self.state();
        instrument!(compio_log::Level::TRACE, "Local::poll", id = ?self.header().id);
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
                    unsafe { MaybeUninit::assume_init_drop(waker) };
                }
                waker.write(abort_on_panic(|| cx.waker().clone()));
                self.header().state.set_has_waker::<Weak, true>();

                Poll::Pending
            });
        }

        unreachable!("Task is completed but has no result")
    }

    fn header(&self) -> &Header {
        unsafe { self.ptr.as_ref() }
    }

    fn state(&self) -> Snapshot {
        self.header().state.load::<Weak>()
    }
}
