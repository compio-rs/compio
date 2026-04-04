//! Waker that carries extra data.

use std::{
    mem::ManuallyDrop,
    pin::Pin,
    ptr,
    sync::Arc,
    task::{Context, Poll, RawWaker, RawWakerVTable, Waker},
};

use compio_send_wrapper::SendWrapper;

pub(crate) trait ExtData: Default {
    type OwnedExt: 'static;

    fn to_owned(&self) -> Self::OwnedExt;

    fn from_owned(owned: &Self::OwnedExt) -> &Self;
}

/// Try to retrieve ext data from the waker and call the callback on it. If ext
/// data can't be retrieved, intialize a dafault on stack and pass a reference
/// of that to `f` instead.
pub(crate) fn with_ext<E, F, R>(waker: &Waker, f: F) -> R
where
    E: ExtData,
    F: FnOnce(&Waker, &E) -> R,
{
    if let Some(ext) = get_ext::<E>(waker) {
        ExtWaker::new(waker, ext).with(|waker| f(waker, ext))
    } else {
        let ext = E::default();
        ExtWaker::new(waker, &ext).with(|waker| f(waker, &ext))
    }
}

/// Remove all [`ExtWaker`] wrapped around the waker and retrieve the underlying
/// waker.
pub(crate) fn get_waker<'a, E: ExtData + 'a>(waker: &'a Waker) -> &'a Waker {
    if ExtWaker::<E>::is(waker) {
        get_waker::<E>(unsafe { ExtWaker::<E>::from_raw(waker.data()) }.waker)
    } else if OwnedExtWaker::<E>::is(waker) {
        get_waker::<E>(&unsafe { OwnedExtWaker::<E>::from_raw(waker.data()) }.waker)
    } else {
        waker
    }
}

pub(crate) fn get_ext<E: ExtData>(waker: &Waker) -> Option<&E> {
    if ExtWaker::<E>::is(waker) {
        unsafe { ExtWaker::from_raw(waker.data()) }
            .ext
            .get()
            .copied()
    } else if OwnedExtWaker::<E>::is(waker) {
        let owned = unsafe { OwnedExtWaker::<E>::from_raw(waker.data()) }
            .ext
            .get()?;
        Some(E::from_owned(owned))
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
pub(crate) struct ExtWaker<'a, E> {
    waker: &'a Waker,
    // `SendWrapper<&Ext>` will not panic when being dropped on other thread since references
    // doesn't need drop
    ext: SendWrapper<&'a E>,
}

impl<'a, E: ExtData> ExtWaker<'a, E> {
    const VTABLE: &'static RawWakerVTable =
        &RawWakerVTable::new(Self::clone, Self::wake, Self::wake_by_ref, Self::drop);

    pub(crate) fn is(waker: &Waker) -> bool {
        ptr::eq(waker.vtable(), Self::VTABLE)
    }

    pub fn new(waker: &'a Waker, ext: &'a E) -> Self {
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

    unsafe fn from_raw<'b>(ptr: *const ()) -> &'b Self {
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
        unsafe { Self::from_raw(ptr) }.waker.wake_by_ref();
    }

    unsafe fn drop(_: *const ()) {
        // `ExtWaker` only contains reference, no need to drop.
    }

    fn to_owned(&self) -> Option<OwnedExtWaker<E>> {
        let ext_data = self.ext.get().copied()?.to_owned();
        let ext = ManuallyDrop::new(SendWrapper::new(ext_data));
        Some(OwnedExtWaker(Arc::new(Inner {
            waker: self.waker.clone(),
            ext,
        })))
    }
}

struct OwnedExtWaker<E: ExtData>(Arc<Inner<E>>);

struct Inner<E: ExtData> {
    waker: Waker,
    ext: ManuallyDrop<SendWrapper<E::OwnedExt>>,
}

impl<E: ExtData> Drop for Inner<E> {
    fn drop(&mut self) {
        if self.ext.valid() {
            unsafe { ManuallyDrop::drop(&mut self.ext) };
        }
    }
}

impl<E: ExtData> OwnedExtWaker<E> {
    const VTABLE: &'static RawWakerVTable =
        &RawWakerVTable::new(Self::clone, Self::wake, Self::wake_by_ref, Self::drop);

    pub(crate) fn is(waker: &Waker) -> bool {
        ptr::eq(waker.vtable(), Self::VTABLE)
    }

    unsafe fn clone(ptr: *const ()) -> RawWaker {
        unsafe { Arc::increment_strong_count(ptr.cast::<Inner<E>>()) };
        RawWaker::new(ptr, Self::VTABLE)
    }

    unsafe fn wake(ptr: *const ()) {
        unsafe { Arc::from_raw(ptr.cast::<Inner<E>>()) }
            .waker
            .wake_by_ref();
    }

    unsafe fn wake_by_ref(ptr: *const ()) {
        unsafe { Self::from_raw(ptr) }.waker.wake_by_ref();
    }

    unsafe fn drop(ptr: *const ()) {
        _ = unsafe { Arc::from_raw(ptr.cast::<Inner<E>>()) };
    }

    unsafe fn from_raw<'b>(ptr: *const ()) -> &'b Inner<E> {
        unsafe { &*ptr.cast::<Inner<E>>() }
    }

    fn into_std(self) -> Waker {
        unsafe {
            Waker::from_raw(RawWaker::new(
                Arc::into_raw(self.0).cast::<()>(),
                OwnedExtWaker::<E>::VTABLE,
            ))
        }
    }
}
