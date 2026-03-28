#![allow(dead_code)]

use std::{
    fmt::{self, Debug},
    hash::Hash,
    io,
    mem::{self, ManuallyDrop},
    ops::{Deref, DerefMut},
    task::Waker,
};

use compio_buf::{BufResult, IntoInner};
use compio_send_wrapper::SendWrapper;
use thin_cell::unsync::{Inner, Ref, ThinCell};

use crate::{Carry, DriverType, Extra, OpCode, PushEntry, control::Carrier};

/// An operation with other needed information.
///
/// You should not use `RawOp` directly. Instead, use [`Key`] to manage the
/// reference-counted pointer to it.
#[repr(C)]
pub(crate) struct RawOp<M: ?Sized> {
    // Platform-specific extra data.
    //
    // - On Windows, it holds the `OVERLAPPED` buffer and a pointer to the driver.
    // - On Linux with `io_uring`, it holds the flags returned by kernel.
    // - On other platforms, it stores tracker for multi-fd `OpCode`s.
    //
    // Extra MUST be the first field to guarantee the layout for casting on windows. An invariant
    // on IOCP driver is that `RawOp` pointer is the same as `OVERLAPPED` pointer.
    extra: Extra,
    // The cancelled flag indicates the op has been cancelled.
    cancelled: bool,
    result: PushEntry<Option<Waker>, io::Result<usize>>,
    pub(crate) carrier: M,
}

impl<C: ?Sized> RawOp<C> {
    pub fn extra(&self) -> &Extra {
        &self.extra
    }

    pub fn extra_mut(&mut self) -> &mut Extra {
        &mut self.extra
    }

    #[cfg(io_uring)]
    pub fn wake_by_ref(&mut self) {
        if let PushEntry::Pending(Some(w)) = &self.result {
            w.wake_by_ref();
        }
    }
}

#[cfg(windows)]
impl<C: crate::Carry + ?Sized> RawOp<C> {
    /// Call [`OpCode::operate`] and assume that it is not an overlapped op,
    /// which means it never returns [`Poll::Pending`].
    ///
    /// [`Poll::Pending`]: std::task::Poll::Pending
    pub fn operate_blocking(&mut self) -> io::Result<usize> {
        use std::task::Poll;

        let optr = self.extra_mut().optr();
        let res = unsafe { self.carrier.operate(optr.cast()) };
        match res {
            Poll::Pending => unreachable!("this operation is not overlapped"),
            Poll::Ready(res) => res,
        }
    }
}

impl<C: ?Sized> Debug for RawOp<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RawOp")
            .field("extra", &self.extra)
            .field("cancelled", &self.cancelled)
            .field("result", &self.result)
            .field("Carrier", &"<...>")
            .finish()
    }
}

/// A typed wrapper for key of Ops submitted into driver.
#[repr(transparent)]
pub struct Key<T> {
    erased: ErasedKey,
    _p: std::marker::PhantomData<T>,
}

impl<T> Unpin for Key<T> {}

impl<T> Clone for Key<T> {
    fn clone(&self) -> Self {
        Self {
            erased: self.erased.clone(),
            _p: std::marker::PhantomData,
        }
    }
}

impl<T> Debug for Key<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Key({})", self.erased.inner.as_ptr() as usize)
    }
}

impl<T> Key<T> {
    pub(crate) fn into_raw(self) -> usize {
        self.erased.into_raw()
    }

    pub(crate) fn erase(self) -> ErasedKey {
        self.erased
    }
}

impl<T: OpCode> Key<T> {
    /// Take the inner result if it is completed.
    ///
    /// # Panics
    ///
    /// Panics if the result is not ready or the `Key` is not unique (multiple
    /// references or borrowed).
    pub(crate) fn take_result(self) -> BufResult<usize, T> {
        // SAFETY: `Key` invariant guarantees that `T` is the actual concrete type.
        unsafe { self.erased.take_result::<T>() }
    }
}

impl<T: OpCode + 'static> Key<T> {
    /// Create [`RawOp`] and get the [`Key`] to it.
    pub(crate) fn new(op: T, extra: impl Into<Extra>, driver_ty: DriverType) -> Self {
        let erased = ErasedKey::new(op, extra.into(), driver_ty);

        Self {
            erased,
            _p: std::marker::PhantomData,
        }
    }

    pub(crate) fn set_extra(&self, extra: impl Into<Extra>) {
        self.borrow().extra = extra.into();
    }
}

impl<T> Deref for Key<T> {
    type Target = ErasedKey;

    fn deref(&self) -> &Self::Target {
        &self.erased
    }
}

impl<T> DerefMut for Key<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.erased
    }
}

/// A type-erased reference-counted pointer to an operation.
///
/// Internally, it uses [`ThinCell`] to manage the reference count and borrowing
/// state. It provides methods to manipulate the underlying operation, such as
/// setting results, checking completion status, and cancelling the operation.
#[derive(Clone)]
#[repr(transparent)]
pub struct ErasedKey {
    inner: ThinCell<RawOp<dyn Carry>>,
}

impl PartialEq for ErasedKey {
    fn eq(&self, other: &Self) -> bool {
        self.inner.ptr_eq(&other.inner)
    }
}

impl Eq for ErasedKey {}

impl Hash for ErasedKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (self.inner.as_ptr() as usize).hash(state)
    }
}

impl Unpin for ErasedKey {}

impl std::borrow::Borrow<usize> for ErasedKey {
    fn borrow(&self) -> &usize {
        // SAFETY: `ThinCell` guarantees to be the same as a thin pointer (one `usize`)
        unsafe { std::mem::transmute(&self.inner) }
    }
}

impl ErasedKey {
    /// Create [`RawOp`] and get the [`ErasedKey`] to it.
    pub(crate) fn new<T: OpCode + 'static>(op: T, extra: Extra, driver_ty: DriverType) -> Self {
        let raw_op = RawOp {
            extra,
            cancelled: false,
            result: PushEntry::Pending(None),
            // SAFETY: carrier is initialized below
            carrier: unsafe { Carrier::new_uninit(op) },
        };
        let mut inner = ThinCell::new(raw_op);
        // SAFETY:
        // - ThinCell is just created, there will be no shared owner or borrower
        // - Carrier is being pinned by ThinCell, it will have a stable address until
        //   move out
        unsafe { inner.borrow_unchecked().carrier.init(driver_ty) };
        Self {
            inner: unsafe { inner.unsize(|p| p as *const Inner<RawOp<dyn Carry>>) },
        }
    }

    /// Create from `user_data` pointer.
    ///
    /// # Safety
    ///
    /// `user_data` must be a valid pointer to `RawOp<dyn OpCode>` previously
    /// created by [`Key::into_raw`].
    pub(crate) unsafe fn from_raw(user_data: usize) -> Self {
        let inner = unsafe { ThinCell::from_raw(user_data as *mut ()) };
        Self { inner }
    }

    /// Create from `Overlapped` pointer.
    ///
    /// # Safety
    ///
    /// `optr` must be a valid pointer to `Overlapped` stored in `Extra` of
    /// `RawOp<dyn OpCode>`.
    #[cfg(windows)]
    pub(crate) unsafe fn from_optr(optr: *mut crate::sys::Overlapped) -> Self {
        let ptr = unsafe { optr.cast::<usize>().offset(-2).cast() };
        let inner = unsafe { ThinCell::from_raw(ptr) };
        Self { inner }
    }

    /// Leak self into a pointer to `Overlapped`.
    #[cfg(windows)]
    pub(crate) fn into_optr(self) -> *mut crate::sys::Overlapped {
        unsafe { self.inner.leak().cast::<usize>().add(2).cast() }
    }

    /// Get the pointer as `user_data`.
    ///
    /// **Do not** call [`from_raw`](Self::from_raw) on the returned value of
    /// this method.
    pub(crate) fn as_raw(&self) -> usize {
        self.inner.as_ptr() as _
    }

    /// Leak self and get the pointer as `user_data`.
    pub(crate) fn into_raw(self) -> usize {
        self.inner.leak() as _
    }

    #[inline]
    pub(crate) fn borrow(&self) -> Ref<'_, RawOp<dyn Carry>> {
        self.inner.borrow()
    }

    /// Set the `cancelled` flag, returning whether it was already cancelled.
    pub(crate) fn set_cancelled(&self) -> bool {
        let mut op = self.borrow();
        mem::replace(&mut op.cancelled, true)
    }

    /// Whether the op is completed.
    pub(crate) fn has_result(&self) -> bool {
        self.borrow().result.is_ready()
    }

    /// Whether the key is uniquely owned.
    pub(crate) fn is_unique(&self) -> bool {
        ThinCell::count(&self.inner) == 1
    }

    /// Complete the op and wake up the future if a waker is set.
    pub(crate) fn set_result(&self, res: io::Result<usize>) {
        let mut this = self.borrow();
        {
            let this = &mut *this;
            #[cfg(not(fusion))]
            unsafe {
                this.carrier.set_result(&res, &this.extra);
            }
            #[cfg(fusion)]
            unsafe {
                crate::sys::Carry::set_result(&mut this.carrier, &res, &this.extra);
            }
        }
        if let PushEntry::Pending(Some(w)) =
            std::mem::replace(&mut this.result, PushEntry::Ready(res))
        {
            w.wake();
        }
    }

    /// Swap the inner [`Extra`] with the provided one, returning the previous
    /// value.
    pub(crate) fn swap_extra(&self, extra: Extra) -> Extra {
        std::mem::replace(&mut self.borrow().extra, extra)
    }

    /// Set waker of the current future.
    pub(crate) fn set_waker(&self, waker: &Waker) {
        let PushEntry::Pending(w) = &mut self.borrow().result else {
            return;
        };

        if w.as_ref().is_some_and(|w| w.will_wake(waker)) {
            return;
        }

        *w = Some(waker.clone());
    }

    /// Take the inner result if it is completed.
    ///
    /// # Safety
    ///
    /// `T` must be the actual concrete type of the `Key`.
    ///
    /// # Panics
    ///
    /// Panics if the result is not ready or the `Key` is not unique (multiple
    /// references or borrowed).
    unsafe fn take_result<T: OpCode>(self) -> BufResult<usize, T> {
        // SAFETY: Caller guarantees that `T` is the actual concrete type.
        let this = unsafe { self.inner.downcast_unchecked::<RawOp<Carrier<T>>>() };
        let op = this.try_unwrap().map_err(|_| ()).expect("Key not unique");
        let res = op.result.take_ready().expect("Result not ready");
        BufResult(res, op.carrier.into_inner())
    }

    /// Unsafely freeze the `Key` by bypassing borrow flag of [`ThinCell`],
    /// preventing it from being dropped and unconditionally expose the
    /// underlying `RawOp<dyn OpCode>`.
    ///
    /// # Safety
    /// - During the time the [`FrozenKey`] is alive, no other references to the
    ///   underlying `RawOp<dyn OpCode>` is used.
    /// - One must not touch [`ThinCell`]'s internal state at all, as `Cell` is
    ///   strictly single-threaded. This means no borrowing, no cloning, no
    ///   dropping, etc.
    pub(crate) unsafe fn freeze(self) -> FrozenKey {
        FrozenKey {
            inner: ManuallyDrop::new(self),
            thread_id: SendWrapper::new(()),
        }
    }
}

impl Debug for ErasedKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ErasedKey({})", self.inner.as_ptr() as usize)
    }
}

/// A frozen view into a [`Key`].
///
/// It's guaranteed to have the same layout as [`ErasedKey`].
#[repr(C)]
pub(crate) struct FrozenKey {
    inner: ManuallyDrop<ErasedKey>,
    thread_id: SendWrapper<()>,
}

impl FrozenKey {
    pub fn as_mut(&mut self) -> &mut RawOp<dyn Carry> {
        unsafe { self.inner.inner.borrow_unchecked() }
    }

    pub fn into_inner(self) -> ErasedKey {
        let mut this = ManuallyDrop::new(self);
        unsafe { ManuallyDrop::take(&mut this.inner) }
    }
}

impl Drop for FrozenKey {
    fn drop(&mut self) {
        if self.thread_id.valid() {
            unsafe { ManuallyDrop::drop(&mut self.inner) }
        }
    }
}

unsafe impl Send for FrozenKey {}
unsafe impl Sync for FrozenKey {}

/// A temporary view into a [`Key`].
///
/// It is mainly used in the driver to avoid accidentally decreasing the
/// reference count of the `Key` when the driver is not completed and may still
/// emit event with the `user_data`.
pub(crate) struct BorrowedKey(ManuallyDrop<ErasedKey>);

impl BorrowedKey {
    pub unsafe fn from_raw(user_data: usize) -> Self {
        let key = unsafe { ErasedKey::from_raw(user_data) };
        Self(ManuallyDrop::new(key))
    }

    pub fn upgrade(self) -> ErasedKey {
        ManuallyDrop::into_inner(self.0)
    }
}

impl Deref for BorrowedKey {
    type Target = ErasedKey;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[cfg(test)]
mod test {
    use std::borrow::Borrow;

    use compio_buf::BufResult;

    use crate::{DriverType, Proactor, key::ErasedKey, op::Asyncify};

    #[test]
    fn test_key_borrow() {
        let driver = Proactor::new().unwrap();
        let extra = driver.default_extra();
        let key = ErasedKey::new(
            Asyncify::new(|| BufResult(Ok(0), [0u8])),
            extra,
            DriverType::Poll,
        );
        assert_eq!(&key.as_raw(), Borrow::<usize>::borrow(&key));
    }
}
