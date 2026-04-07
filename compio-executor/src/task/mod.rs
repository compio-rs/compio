use std::{
    array,
    mem::{ManuallyDrop, MaybeUninit, offset_of},
    panic::{AssertUnwindSafe, catch_unwind},
    pin::Pin,
    ptr::{self, NonNull, drop_in_place},
    task::{Context, Poll, Waker, ready},
};

use compio_log::{debug, instrument, trace};
use compio_send_wrapper::SendWrapper;

use crate::{
    PanicResult, Shared, UnsafeCell,
    queue::TaskId,
    task::{
        local::Local,
        remote::Remote,
        state::{Snapshot, State, Strong},
    },
    util::transpose,
};

mod local;
mod remote;
mod state;

/// A reference counter pointer to the [`TaskAlloc`].
#[derive(Debug)]
#[repr(transparent)]
pub(crate) struct Task(NonNull<Header>);

/// Allocated part of a task, which contains the future, result and all
/// necessary metadata. A pointer to `TaskAlloc` is always a valid pointer to a
/// `Header`.
#[repr(C)]
struct TaskAlloc<F: Future> {
    header: Header,
    future: UnsafeCell<FutureState<F>>,
}

struct Header {
    id: TaskId,
    state: State,
    vtable: &'static TaskVtable,
    tracker: ManuallyDrop<SendWrapper<()>>,
    shared: UnsafeCell<*const Shared>,
    waker: UnsafeCell<MaybeUninit<Waker>>,
}

union FutureState<F: Future> {
    future: ManuallyDrop<F>,
    result: ManuallyDrop<PanicResult<F::Output>>,
}

struct TaskVtable {
    dealloc: unsafe fn(NonNull<Header>),
    run_future: unsafe fn(NonNull<Header>, &mut Context<'_>) -> Poll<()>,
    take_result: unsafe fn(NonNull<Header>, NonNull<()>),
    drop_future: unsafe fn(NonNull<Header>, bool),
}

impl<F: Future + 'static> TaskAlloc<F> {
    const FUT_OFFSET: usize = offset_of!(Self, future);
    const VTABLE: &'static TaskVtable = &TaskVtable {
        dealloc: Self::dealloc,
        run_future: Self::run_future,
        take_result: Self::take_result,
        drop_future: Self::drop_future,
    };

    fn future_cell(header: NonNull<Header>) -> &'static UnsafeCell<FutureState<F>> {
        unsafe {
            &*header
                .byte_add(Self::FUT_OFFSET)
                .cast::<UnsafeCell<FutureState<F>>>()
                .as_ptr()
        }
    }

    unsafe fn run_future(header: NonNull<Header>, cx: &mut Context<'_>) -> Poll<()> {
        let future_cell = Self::future_cell(header);

        // SAFETY:
        // - The caller guarantees that we're pinned
        // - The caller guarantees that we're in the `future` state
        let res = ready!(future_cell.with_mut(|fut_ptr| {
            let fut = unsafe { Pin::new_unchecked(&mut *(*fut_ptr).future) };
            transpose(catch_unwind(AssertUnwindSafe(|| fut.poll(cx))))
        }));

        // SAFETY: The caller guarantees that we're in the `future` state and are on the
        // same thread as the future is created, so it's safe to drop the future
        future_cell.with_mut(|fut_ptr| {
            unsafe { drop_in_place(fut_ptr as *mut F) };
            let new_state = FutureState {
                result: ManuallyDrop::new(res),
            };
            unsafe { ptr::write(fut_ptr, new_state) };
        });

        Poll::Ready(())
    }

    unsafe fn take_result(header: NonNull<Header>, target: NonNull<()>) {
        let future_cell = Self::future_cell(header);

        // SAFETY:
        // - The caller guarantees that we're in the `result` state and guarantees if
        //   the result type is not multithread-safe, this is called on the same thread
        //   as the future is created.
        // - The caller guarantees that the target pointer is valid for writes and
        //   properly aligned for `PanicResult<F::Output>`.
        future_cell.with(|fut_ptr| {
            let fut_ptr = fut_ptr as *const PanicResult<F::Output>;
            unsafe { std::ptr::copy_nonoverlapping(fut_ptr, target.as_ptr().cast(), 1) };
        });
    }

    unsafe fn drop_future(header: NonNull<Header>, has_result: bool) {
        let future_cell = Self::future_cell(header);

        future_cell.with_mut(|fut_ptr| {
            if has_result {
                unsafe { drop_in_place::<PanicResult<F::Output>>(fut_ptr as _) };
            } else {
                unsafe { drop_in_place::<F>(fut_ptr as _) };
            }
        });
    }

    unsafe fn dealloc(header: NonNull<Header>) {
        // SAFETY: The caller guarantees that the pointer is valid and properly aligned
        // for `TaskAlloc<F>`, and that no other reference to the allocation
        // exists.
        drop(unsafe { Box::from_raw(header.as_ptr().cast::<TaskAlloc<F>>()) });
    }
}

impl Task {
    pub fn new<F: Future + 'static, const N: usize>(
        id: TaskId,
        shared: NonNull<Shared>,
        tracker: SendWrapper<()>,
        future: F,
    ) -> [Task; N] {
        let alloc = Box::new(TaskAlloc {
            header: Header {
                id,
                state: State::new::<N>(),
                vtable: TaskAlloc::<F>::VTABLE,
                tracker: ManuallyDrop::new(tracker),
                shared: UnsafeCell::new(shared.as_ptr()),
                waker: UnsafeCell::new(MaybeUninit::uninit()),
            },
            future: UnsafeCell::new(FutureState {
                future: ManuallyDrop::new(future),
            }),
        });

        // SAFETY: The pointer was returned by `Box`, which guarantees that it's
        // non-null and properly aligned.
        let ptr = unsafe { NonNull::new_unchecked(Box::into_raw(alloc) as _) };

        array::from_fn(|_| Task(ptr))
    }

    pub unsafe fn from_raw(ptr: *const ()) -> Self {
        Self(NonNull::new(ptr as *mut () as _).expect("Invalid pointer"))
    }

    pub fn as_raw(&self) -> *const () {
        self.0.as_ptr() as _
    }

    pub unsafe fn increment_count(ptr: *const ()) {
        unsafe { &*(ptr as *const Header) }.state.inc();
    }

    pub fn schedule(&self) {
        match self.view() {
            Ok(local) => local.schedule(),
            Err(remote) => remote.schedule(),
        }
    }

    /// Cancel the task.
    ///
    /// If `drop_result` is true, the result will be dropped if it exists.
    pub fn cancel(&self, drop_result: bool) {
        let header = self.header();
        self.schedule();
        let state = header.state.set_cancelled();
        if drop_result && state.has_result() {
            header.state.set_has_result::<Strong, false>();
            unsafe { (header.vtable.drop_future)(self.0, true) }
        }
    }

    /// # Safety
    ///
    /// This function can only be called by `Executor` and the task must not be
    /// in completed state.
    pub unsafe fn run(&self) -> Poll<()> {
        instrument!(compio_log::Level::TRACE, "Task::run", id = ?self.header().id);

        let header = self.header();
        let state = header.state.set_scheduled::<false>();
        if state.is_cancelled() {
            debug!(?state, "Cancelled");
            return Poll::Ready(());
        }

        self.with_waker(|waker| {
            let ctx = &mut Context::from_waker(waker);
            let res = unsafe { (header.vtable.run_future)(self.0, ctx) };
            if res.is_ready() {
                let state = header.state.set_finished_running();
                debug!(?state, "Finished");
                if state.has_waker() {
                    header.state.set_has_waker::<Strong, false>();
                    header
                        .waker
                        .with_mut(|ptr| unsafe { (*ptr).assume_init_read() }.wake());
                }
            }
            trace!("Pending");
            res
        })
    }

    /// # Safety
    ///
    /// This function can only be called by `JoinHandle`.
    pub unsafe fn poll<T>(&self, cx: &mut Context<'_>) -> Poll<Option<PanicResult<T>>> {
        match self.view() {
            Ok(local) => unsafe { local.poll(cx) },
            Err(remote) => unsafe { remote.poll(cx) },
        }
    }

    // Drop everything inside the allocation, but do not deallocate the memory. This
    // is used when the task is completed or cancelled, and we want to drop the
    // future, result and/or waker, but the memory will be deallocated when the
    // reference count reaches 0.
    //
    // If `unset_shared` is true, this will also reset task's `Shared` pointer to
    // null so that `Executor` can drop `Shared` safely.
    //
    // # Safety
    //
    // Can only be called by the Executor once.
    pub unsafe fn drop(&self, unset_shared: bool) {
        instrument!(compio_log::Level::TRACE, "Task::drop", id = ?self.header().id);
        let header = self.header();
        debug_assert!(
            header.tracker.valid(),
            "drop_future should only be called by Executor"
        );
        let state = self.state();

        if !state.is_completed() {
            // The task is finished without result, drop future
            unsafe { (header.vtable.drop_future)(self.0, false) };
        }

        // If `JoinHandle` is setting the waker, it'll check the state afterwards and
        // drop the waker. Otherwise, we drop the waker here if it exists.
        if state.has_waker() && !state.is_setting_waker() {
            header
                .waker
                .with_mut(|ptr| unsafe { drop_in_place(ptr.cast::<Waker>()) });
        }

        let mut state = header.state.set_dropped();

        if !unset_shared {
            return;
        }

        // SAFETY: We have set the cancelled bit in `set_dropped`, so concurrent access
        // to the pointer will be stopped.
        unsafe { header.shared.with_mut(|p| ptr::write(p, ptr::null_mut())) };

        // Wait for scheduling to stop as they're accessing `Shared`.
        while state.is_scheduling() {
            crate::yield_now();
            state = header.state.load::<Strong>();
        }
    }

    pub fn state(&self) -> Snapshot {
        self.header().state.load::<Strong>()
    }

    fn header(&self) -> &Header {
        unsafe { self.0.as_ref() }
    }

    #[inline(always)]
    fn view(&self) -> Result<Local<'_>, Remote<'_>> {
        if self.header().tracker.valid() {
            // SAFETY: We have checked that the tracker is valid, so this must be the same
            // thread as the task allocation is created on.
            Ok(unsafe { Local::new(self.0) })
        } else {
            Err(Remote::new(self.0))
        }
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        let header = self.header();
        let state = header.state.dec();
        trace!(old = ?state, "Task dropped");
        if state.count() > 1 {
            return;
        };

        println!("{state:?}");
        debug_assert!(state.is_completed() | state.is_cancelled());
        debug_assert!(!state.is_setting_waker());
        debug_assert!(!state.has_waker());

        // If the result is still present, drop it now
        // This happens when JoinHandle was dropped/detached without taking the result
        if state.has_result() {
            unsafe { (header.vtable.drop_future)(self.0, true) };
        }

        trace!("Task deallocated");
        // SAFETY: We have checked that the reference count is 0, so no other reference
        // to the allocation exists and we can safely deallocate it; and deallocation is
        // thread-safe since we're not touching anything inside (dropping).
        unsafe { (header.vtable.dealloc)(self.0) }
    }
}

#[cfg(test)]
mod test {
    use std::mem::needs_drop;

    use super::*;

    struct NeedsDrop {
        _str: String,
    }

    impl Future for NeedsDrop {
        type Output = ();

        fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
            Poll::Ready(())
        }
    }

    /// All dropping is handled manually by [`Executor`]. The memory is
    /// deallocated by [`Task`].
    ///
    /// [`Executor`]: crate::Executor
    const _: () = assert!(!needs_drop::<TaskAlloc<NeedsDrop>>());
}
