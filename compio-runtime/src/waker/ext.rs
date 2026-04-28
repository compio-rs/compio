//! Waker that carries extra data.

use std::{
    mem::ManuallyDrop,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, RawWaker, RawWakerVTable, Waker},
};

use compio_send_wrapper::SendWrapper;

use crate::Ext;

/// Try to retrieve ext data from the waker and call the callback on it. If ext
/// data can't be retrieved, initialize a dafault on the stack and pass a
/// reference of that to `f` instead.
pub(crate) fn with_ext<F, R>(waker: &Waker, f: F) -> R
where
    F: FnOnce(&Waker, &Ext) -> R,
{
    if let Some(ext) = get_ext(waker) {
        f(waker, ext)
    } else {
        let ext = Ext::default();
        f(waker, &ext)
    }
}

/// Remove all [`ExtWaker`] wrapped around the waker and retrieve the underlying
/// waker.
pub(crate) fn get_waker(waker: &Waker) -> &Waker {
    if waker.vtable() == ExtWaker::VTABLE {
        get_waker(unsafe { ExtWaker::from_raw(waker.data()) }.waker)
    } else if waker.vtable() == OwnedExtWaker::VTABLE {
        get_waker(&unsafe { OwnedExtWaker::from_raw(waker.data()) }.waker)
    } else {
        waker
    }
}

pub(crate) fn get_ext(waker: &Waker) -> Option<&Ext<'_>> {
    if waker.vtable() == ExtWaker::VTABLE {
        unsafe { ExtWaker::from_raw(waker.data()) }
            .ext
            .get()
            .copied()
    } else if waker.vtable() == OwnedExtWaker::VTABLE {
        unsafe { OwnedExtWaker::from_raw(waker.data()) }.ext.get()
    } else {
        None
    }
}

/// [`Waker`] with extra data associated.
///
/// When cloned in the same thread where it's created, the extra data is cloned
/// into owned form and converted to [`OwnedExtWaker`]; otherwise, only the
/// underlying waker is cloned and the data will be lost.
#[derive(Debug, Clone)]
pub(crate) struct ExtWaker<'a, 'b> {
    waker: &'a Waker,
    // `SendWrapper<&Ext>` will not panic when being dropped on other thread since references
    // doesn't need drop
    ext: SendWrapper<&'a Ext<'b>>,
}

impl<'a, 'b> ExtWaker<'a, 'b> {
    const VTABLE: &'static RawWakerVTable =
        &RawWakerVTable::new(Self::clone, Self::wake, Self::wake_by_ref, Self::drop);

    pub fn new(waker: &'a Waker, ext: &'a Ext<'b>) -> Self {
        Self {
            waker,
            ext: SendWrapper::new(ext),
        }
    }

    pub fn poll<F: Future + ?Sized>(&self, fut: Pin<&mut F>) -> Poll<F::Output> {
        self.with(|waker| fut.poll(&mut Context::from_waker(waker)))
    }

    fn with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&Waker) -> R,
    {
        let waker = unsafe { Waker::new(self as *const _ as *const (), Self::VTABLE) };
        f(&waker)
    }

    fn should_notify(&self) -> bool {
        if let Some(ext) = self.ext.get() {
            ext.should_notify_always()
        } else {
            true
        }
    }

    unsafe fn from_raw<'s>(ptr: *const ()) -> &'s Self {
        unsafe { &*ptr.cast::<Self>() }
    }

    unsafe fn clone(ptr: *const ()) -> RawWaker {
        let this = unsafe { Self::from_raw(ptr) };

        if let Some(owned) = this.to_owned() {
            let waker = ManuallyDrop::new(owned.into_std());
            RawWaker::new(waker.data(), waker.vtable())
        } else {
            let waker = ManuallyDrop::new(this.waker.clone());
            RawWaker::new(waker.data(), waker.vtable())
        }
    }

    unsafe fn wake(_: *const ()) {
        unreachable!("ExtWaker will only be accessed with reference")
    }

    unsafe fn wake_by_ref(ptr: *const ()) {
        let this = unsafe { Self::from_raw(ptr) };
        if this.should_notify() {
            this.waker.wake_by_ref();
        }
    }

    unsafe fn drop(_: *const ()) {
        // `ExtWaker` only contains reference, no need to drop.
    }

    fn to_owned(&self) -> Option<OwnedExtWaker> {
        let ext_data = self.ext.get().copied()?.to_owned();
        let ext = ManuallyDrop::new(SendWrapper::new(ext_data));
        Some(OwnedExtWaker(Arc::new(Inner {
            waker: self.waker.clone(),
            ext,
        })))
    }
}

struct OwnedExtWaker(Arc<Inner>);

struct Inner {
    waker: Waker,
    ext: ManuallyDrop<SendWrapper<Ext<'static>>>,
}

impl Inner {
    fn should_notify(&self) -> bool {
        if let Some(ext) = self.ext.get() {
            ext.should_notify_always()
        } else {
            true
        }
    }

    fn wake_by_ref(&self) {
        if self.should_notify() {
            self.waker.wake_by_ref();
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        if self.ext.valid() {
            unsafe { ManuallyDrop::drop(&mut self.ext) };
        }
    }
}

impl OwnedExtWaker {
    const VTABLE: &'static RawWakerVTable =
        &RawWakerVTable::new(Self::clone, Self::wake, Self::wake_by_ref, Self::drop);

    unsafe fn clone(ptr: *const ()) -> RawWaker {
        unsafe { Arc::increment_strong_count(ptr.cast::<Inner>()) };
        RawWaker::new(ptr, Self::VTABLE)
    }

    unsafe fn wake(ptr: *const ()) {
        unsafe { Arc::from_raw(ptr.cast::<Inner>()) }.wake_by_ref();
    }

    unsafe fn wake_by_ref(ptr: *const ()) {
        unsafe { Self::from_raw(ptr) }.wake_by_ref();
    }

    unsafe fn drop(ptr: *const ()) {
        _ = unsafe { Arc::from_raw(ptr.cast::<Inner>()) };
    }

    unsafe fn from_raw<'b>(ptr: *const ()) -> &'b Inner {
        unsafe { &*ptr.cast::<Inner>() }
    }

    fn into_std(self) -> Waker {
        unsafe {
            Waker::from_raw(RawWaker::new(
                Arc::into_raw(self.0).cast::<()>(),
                OwnedExtWaker::VTABLE,
            ))
        }
    }
}
