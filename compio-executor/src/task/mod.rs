use std::{
    array,
    cell::UnsafeCell,
    mem::{ManuallyDrop, MaybeUninit, offset_of},
    panic::{AssertUnwindSafe, catch_unwind},
    pin::Pin,
    ptr::{self, NonNull},
    task::{Context, Poll, Waker, ready},
};

use compio_log::{debug, instrument, trace};
use compio_send_wrapper::SendWrapper;

use crate::{
    PanicResult, Shared,
    queue::TaskId,
    task::{
        local::Local,
        remote::Remote,
        state::{State, Strong},
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

struct TaskVtable {
    offset: isize,
    run: unsafe fn(*const (), &mut Context<'_>) -> Poll<()>,
    take_result: unsafe fn(*const (), *mut ()),
    drop: unsafe fn(*const (), bool),
    dealloc: unsafe fn(NonNull<Header>),
}

impl<F: Future + 'static> TaskAlloc<F> {
    const VTABLE: &'static TaskVtable = &TaskVtable {
        offset: offset_of!(TaskAlloc<F>, future) as _,
        run: FutureState::<F>::run,
        take_result: FutureState::<F>::take_result,
        drop: FutureState::<F>::drop,
        dealloc: Self::dealloc,
    };

    unsafe fn dealloc(ptr: NonNull<Header>) {
        // SAFETY: The caller guarantees that the pointer is valid and properly aligned
        // for `TaskAlloc<F>`, and that no other reference to the allocation
        // exists.
        drop(unsafe { Box::from_raw(ptr.as_ptr().cast::<TaskAlloc<F>>()) });
    }
}

struct Header {
    id: TaskId,
    state: State,
    vtable: &'static TaskVtable,
    tracker: ManuallyDrop<SendWrapper<()>>,
    shared: UnsafeCell<*const Shared>,
    waker: UnsafeCell<MaybeUninit<Waker>>,
}

impl Header {
    pub(crate) fn future_ptr(&self) -> *const () {
        let ptr = unsafe { (self as *const Header).byte_offset(self.vtable.offset) };
        ptr as _
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

    pub fn cancel(&self) {
        let header = self.header();
        header.state.set_cancelled();
        self.schedule();
        unsafe { ptr::write(header.shared.get(), ptr::null_mut::<Shared>()) };
    }

    /// # Safety
    ///
    /// This function can only be called by `Executor` and the task must not be
    /// in completed state.
    pub unsafe fn run(&self) -> Poll<()> {
        instrument!(compio_log::Level::TRACE, "Task::run", id = ?self.header().id);
        self.with_std(|waker| {
            let header = self.header();
            let state = header.state.load::<Strong>();
            if state.is_cancelled() {
                debug!(?state, "Cancelled");
                return Poll::Ready(());
            }

            let ctx = &mut Context::from_waker(waker);
            let res = unsafe { (header.vtable.run)(header.future_ptr(), ctx) };
            if res.is_ready() {
                let state = header.state.set_finished_running();
                debug!(?state, "Finished");
                if state.has_waker() {
                    header.state.set_has_waker::<Strong, false>();
                    unsafe { (*header.waker.get()).assume_init_read() }.wake();
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
    // # Safety
    //
    // Can only be called by the Executor once.
    pub unsafe fn drop(&self) {
        instrument!(compio_log::Level::TRACE, "Task::drop", id = ?self.header().id);
        let header = self.header();
        debug_assert!(
            header.tracker.valid(),
            "drop_future should only be called by Executor"
        );
        let state = header.state.load::<Strong>();

        // Don't drop the result here - it will be taken by JoinHandle or dropped in
        // Task's Drop Only drop the future if the task hasn't completed yet
        if !state.is_completed() {
            unsafe { (header.vtable.drop)(header.future_ptr(), false) };
        }

        // If someone else is setting the waker, they'll check the state afterwards and
        // drop the waker. Otherwise, we drop the waker here if it exists.
        if state.has_waker() && !state.is_setting_waker() {
            let waker = unsafe { &mut *header.waker.get() };
            unsafe { MaybeUninit::assume_init_drop(waker) };
        }

        header.state.set_dropped();
        // SAFETY: We have set the cancelled bit in `set_dropped`, so concurrent access
        // to the pointer will be stopped. Setting this is for same-thread access which
        // will not check for cancel flag.
        unsafe { ptr::write(header.shared.get(), ptr::null_mut::<Shared>()) };
    }

    fn header(&self) -> &Header {
        unsafe { self.0.as_ref() }
    }

    #[inline(always)]
    fn view(&self) -> Result<Local<'_>, Remote<'_>> {
        if self.header().tracker.valid() {
            // SAFETY: We have checked that the tracker is valid, so this must be the same
            // thread as the task allocation is created on.
            Ok(unsafe { Local::new(self.header()) })
        } else {
            Err(Remote::new(self.header()))
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

        debug_assert!(state.is_completed() | state.is_cancelled());
        debug_assert!(!state.is_setting_waker());
        debug_assert!(!state.has_waker());

        // If the result is still present, drop it now
        // This happens when JoinHandle was dropped/detached without taking the result
        if state.has_result() {
            unsafe { (header.vtable.drop)(header.future_ptr(), true) };
        }

        trace!("Task deallocated");
        // SAFETY: We have checked that the reference count is 0, so no other reference
        // to the allocation exists and we can safely deallocate it; and deallocation is
        // thread-safe since we're not touching anything inside (dropping).
        unsafe { (header.vtable.dealloc)(self.0) }
    }
}

union FutureState<F: Future> {
    future: ManuallyDrop<F>,
    result: ManuallyDrop<PanicResult<F::Output>>,
}

impl<F: Future + 'static> FutureState<F> {
    unsafe fn run(ptr: *const (), cx: &mut Context<'_>) -> Poll<()> {
        let this = unsafe { &mut *(ptr as *mut Self) };

        // SAFETY:
        // - The caller guarantees that we're pinned
        // - The caller guarantees that we're in the `future` state
        let fut = unsafe { Pin::new_unchecked(&mut *this.future) };
        let res = ready!(transpose(catch_unwind(AssertUnwindSafe(|| fut.poll(cx)))));

        // SAFETY: The caller guarantees that we're in the `future` state and are on the
        // same thread as the future is created, so it's safe to drop the future
        unsafe { ManuallyDrop::drop(&mut this.future) };
        this.result = ManuallyDrop::new(res);

        Poll::Ready(())
    }

    unsafe fn take_result(ptr: *const (), target: *mut ()) {
        let this = unsafe { &mut *(ptr as *mut Self) };

        let src = &raw const this.result;

        // SAFETY:
        // - The caller guarantees that we're in the `result` state and guarantees if
        //   the result type is not multithread-safe, this is called on the same thread
        //   as the future is created.
        // - The caller guarantees that the target pointer is valid for writes and
        //   properly aligned for `PanicResult<F::Output>`.
        unsafe { std::ptr::copy_nonoverlapping(src, target.cast(), 1) };
    }

    unsafe fn drop(ptr: *const (), has_result: bool) {
        let this = unsafe { &mut *(ptr as *mut Self) };

        if has_result {
            unsafe { ManuallyDrop::drop(&mut this.result) };
        } else {
            unsafe { ManuallyDrop::drop(&mut this.future) };
        }
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

    const _: () = assert!(offset_of!(TaskAlloc<NeedsDrop>, future) == size_of::<Header>());
}
