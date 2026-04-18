use std::{
    mem::ManuallyDrop,
    sync::Arc,
    task::{RawWaker, RawWakerVTable, Wake, Waker},
};

pub struct WakerArrayRef<'a, const N: usize>([Option<&'a Waker>; N]);

impl<'a, const N: usize> WakerArrayRef<'a, N> {
    const VTABLE: &'static RawWakerVTable =
        &RawWakerVTable::new(Self::clone, Self::wake, Self::wake_by_ref, Self::drop);

    pub fn new(wakers: [Option<&'a Waker>; N]) -> Self {
        Self(wakers)
    }

    pub fn with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&Waker) -> R,
    {
        let waker = unsafe { Waker::new(self as *const Self as *const (), Self::VTABLE) };
        f(&waker)
    }

    fn wake_impl(&self) {
        for waker in self.0.iter().flatten() {
            waker.wake_by_ref();
        }
    }

    fn to_owned(&self) -> WakerArray<N> {
        WakerArray(self.0.map(|waker| waker.cloned()))
    }

    unsafe fn from_raw<'s>(ptr: *const ()) -> &'s Self {
        unsafe { &*ptr.cast::<Self>() }
    }

    unsafe fn clone(ptr: *const ()) -> RawWaker {
        let this = unsafe { Self::from_raw(ptr) };
        let owned = this.to_owned();
        let waker = ManuallyDrop::new(Waker::from(Arc::new(owned)));
        RawWaker::new(waker.data(), waker.vtable())
    }

    unsafe fn wake(_: *const ()) {
        unreachable!("WakerArrayRef will only be accessed with reference")
    }

    unsafe fn wake_by_ref(ptr: *const ()) {
        unsafe { Self::from_raw(ptr) }.wake_impl();
    }

    unsafe fn drop(_: *const ()) {
        // `WakerArrayRef` only contains reference, no need to drop.
    }
}

struct WakerArray<const N: usize>([Option<Waker>; N]);

impl<const N: usize> Wake for WakerArray<N> {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        for waker in self.0.iter().flatten() {
            waker.wake_by_ref();
        }
    }
}
