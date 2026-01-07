// TODO: We can change to `ThinBox` when it is stabilized.

use std::{
    hash::Hash,
    io,
    marker::PhantomData,
    mem::{ManuallyDrop, MaybeUninit},
    pin::Pin,
    ptr::NonNull,
    task::Waker,
};

use compio_buf::BufResult;

use crate::{Extra, OpCode, PushEntry, RawFd};

/// An operation with other needed information.
///
/// It should be allocated on the heap. The pointer to this struct is used as
/// `user_data`, and on Windows, it is used as the pointer to `OVERLAPPED`.
///
/// You should not use `RawOp` directly. Instead, use [`Key`] to manage the
/// pointer to it. Crucially, a pointer to `RawOp<T>` can be safely cast to
/// `RawOp<()>` guaranteed by `repr(C)`, and vice versa with metadata stored in
/// `RawOp`.
#[repr(C)]
pub(crate) struct RawOp<T: ?Sized> {
    // Platform-specific extra data.
    //
    // - On Windows, it holds the `OVERLAPPED` buffer and a pointer to the driver.
    // - On Linux with `io_uring`, it holds the flags returned by kernel.
    // - On other platforms, it is empty.
    extra: Extra,
    // The cancelled flag and the result here are manual reference counting. The driver holds the
    // strong ref until it completes; the runtime holds the strong ref until the future is
    // dropped.
    cancelled: bool,
    // The metadata in `*mut RawOp<dyn OpCode>`
    metadata: usize,
    result: PushEntry<Option<Waker>, io::Result<usize>>,
    op: T,
}

#[repr(C)]
union OpCodePtrRepr {
    ptr: NonNull<RawOp<dyn OpCode>>,
    components: OpCodePtrComponents,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct OpCodePtrComponents {
    data_pointer: NonNull<RawOp<()>>,
    metadata: usize,
}

fn opcode_metadata<T: OpCode + 'static>() -> usize {
    let mut op = MaybeUninit::<RawOp<T>>::uninit();
    // SAFETY: same as `core::ptr::metadata`.
    unsafe {
        OpCodePtrRepr {
            ptr: NonNull::new(op.as_mut_ptr() as _).expect("ptr to local shouldn't be null"),
        }
        .components
        .metadata
    }
}

const unsafe fn opcode_dyn_mut(
    ptr: NonNull<RawOp<()>>,
    metadata: usize,
) -> NonNull<RawOp<dyn OpCode>> {
    // SAFETY: same as `core::ptr::from_raw_parts_mut`.
    unsafe {
        OpCodePtrRepr {
            components: OpCodePtrComponents {
                data_pointer: ptr,
                metadata,
            },
        }
        .ptr
    }
}

/// A typed wrapper for key of Ops submitted into driver.
///
/// It doesn't free the inner on dropping. Instead, the memory is managed by the
/// proactor. The inner is only freed when:
///
/// 1. The op is completed and the future asks the result. `into_inner` will be
///    called by the proactor.
/// 2. The op is completed and the future cancels it. `into_box` will be called
///    by the proactor.
#[repr(transparent)]
pub struct Key<T: ?Sized> {
    user_data: ManuallyDrop<Box<RawOp<()>>>,
    _p: PhantomData<Box<RawOp<T>>>,
}

impl<T: ?Sized> PartialEq for Key<T> {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.user_data.as_ref(), other.user_data.as_ref())
    }
}

impl<T: ?Sized> Eq for Key<T> {}

impl<T: ?Sized> Hash for Key<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (self.user_data.as_ref() as *const _ as usize).hash(state)
    }
}

impl<T: ?Sized> Unpin for Key<T> {}

impl<T: OpCode + 'static> Key<T> {
    /// Create [`RawOp`] and get the [`Key`] to it.
    pub(crate) fn new(driver: RawFd, op: T) -> Self {
        let raw_op = Box::new(RawOp {
            extra: Extra::new(driver),
            cancelled: false,
            metadata: opcode_metadata::<T>(),
            result: PushEntry::Pending(None),
            op,
        });
        unsafe { Self::new_unchecked(Box::into_raw(raw_op) as _) }
    }
}

impl<T: OpCode> Key<T> {
    /// Convert to a `Key<dyn OpCode>`.
    pub fn into_dyn(self) -> Key<dyn OpCode> {
        Key {
            user_data: self.user_data,
            _p: PhantomData,
        }
    }

    /// As a reference of `Key<dyn OpCode>`.
    pub fn as_dyn(&self) -> &Key<dyn OpCode> {
        // SAFETY: the layout of `Key<T>` and `Key<dyn OpCode>` are the same guaranteed
        // by `repr(transparent)`.
        unsafe { std::mem::transmute(self) }
    }

    /// As a mutable reference of `Key<dyn OpCode>`.
    pub fn as_dyn_mut(&mut self) -> &mut Key<dyn OpCode> {
        // SAFETY: the layout of `Key<T>` and `Key<dyn OpCode>` are the same guaranteed
        // by `repr(transparent)`.
        unsafe { std::mem::transmute(self) }
    }
}

impl<T: ?Sized> Key<T> {
    /// Create a new `Key` with the given user data.
    ///
    /// # Safety
    ///
    /// Caller needs to ensure that `T` does correspond to `user_data` in driver
    /// this `Key` is created with. In most cases, it is enough to let `T` be
    /// `dyn OpCode`.
    pub unsafe fn new_unchecked(user_data: usize) -> Self {
        Self {
            user_data: ManuallyDrop::new(unsafe { Box::from_raw(user_data as _) }),
            _p: PhantomData,
        }
    }

    /// Get the unique user-defined data.
    pub fn user_data(&self) -> usize {
        self.as_non_null().as_ptr() as usize
    }

    fn as_ref(&self) -> &RawOp<()> {
        self.user_data.as_ref()
    }

    fn as_mut(&mut self) -> &mut RawOp<()> {
        self.user_data.as_mut()
    }

    fn as_dyn_op(&mut self) -> &mut RawOp<dyn OpCode> {
        let ptr = self.as_non_null();
        let metadata = self.as_mut().metadata;
        // SAFETY: metadata from `Key::new`.
        unsafe { opcode_dyn_mut(ptr, metadata).as_mut() }
    }

    fn as_non_null(&self) -> NonNull<RawOp<()>> {
        NonNull::from_ref(self.user_data.as_ref())
    }

    fn cast<U>(&self) -> NonNull<RawOp<U>> {
        self.as_non_null().cast()
    }

    /// Take the inner [`Extra`].
    pub(crate) fn take_extra(&mut self) -> Extra {
        std::mem::replace(&mut self.as_mut().extra, Extra::new(RawFd::default()))
    }

    /// Mutable reference to [`Extra`].
    #[allow(dead_code)] // on polling, this is never used
    pub(crate) fn extra_mut(&mut self) -> &mut Extra {
        &mut self.as_mut().extra
    }

    /// Cancel the op, decrease the ref count. The return value indicates if the
    /// op is completed. If so, the op should be dropped because it is
    /// useless.
    pub(crate) fn set_cancelled(&mut self) -> bool {
        self.as_mut().cancelled = true;
        self.has_result()
    }

    /// Complete the op, decrease the ref count. Wake the future if a waker is
    /// set. The return value indicates if the op is cancelled. If so, the
    /// op should be dropped because it is useless.
    pub(crate) fn set_result(&mut self, res: io::Result<usize>) -> bool {
        let this = self.as_dyn_op();
        #[cfg(io_uring)]
        if let Ok(res) = res {
            unsafe {
                Pin::new_unchecked(&mut this.op).set_result(res);
            }
        }
        if let PushEntry::Pending(Some(w)) =
            std::mem::replace(&mut this.result, PushEntry::Ready(res))
        {
            w.wake();
        }
        this.cancelled
    }

    /// Whether the op is completed.
    pub(crate) fn has_result(&self) -> bool {
        self.as_ref().result.is_ready()
    }

    /// Set waker of the current future.
    pub(crate) fn set_waker(&mut self, waker: Waker) {
        if let PushEntry::Pending(w) = &mut self.as_mut().result {
            *w = Some(waker)
        }
    }

    /// Get the inner [`RawOp`]. It is usually used to drop the inner
    /// immediately, without knowing about the inner `T`.
    ///
    /// # Safety
    ///
    /// Call it only when the op is cancelled and completed, which is the case
    /// when the ref count becomes zero. See doc of [`Key::set_cancelled`]
    /// and [`Key::set_result`].
    pub(crate) unsafe fn into_box(mut self) -> Box<RawOp<dyn OpCode>> {
        // SAFETY: user_data is created as `Box<RawOp<T>>`, which is fine to be casted
        // to `Box<RawOp<dyn OpCode>>`.
        unsafe { Box::from_raw(self.as_dyn_op()) }
    }
}

impl<T> Key<T> {
    /// Take the inner result if it is completed.
    ///
    /// # Safety
    ///
    /// Call it only when the op is completed, otherwise it is UB.
    pub(crate) unsafe fn take_result(self) -> BufResult<usize, T> {
        // TODO(George-Miao): use `Box::from_non_null` when `box_vec_non_null` is
        // stablized
        let op = unsafe { Box::from_raw(self.cast::<T>().as_ptr()) };
        BufResult(unsafe { op.result.take_ready().unwrap_unchecked() }, op.op)
    }
}

impl<T: OpCode + ?Sized> Key<T> {
    /// Pin the inner op.
    pub(crate) fn as_pinned_op(&mut self) -> Pin<&mut dyn OpCode> {
        let this = self.as_dyn_op();
        // SAFETY: the inner is pinned with Box.
        unsafe { Pin::new_unchecked(&mut this.op) }
    }

    /// Call [`OpCode::operate`] and assume that it is not an overlapped op,
    /// which means it never returns [`Poll::Pending`].
    ///
    /// [`Poll::Pending`]: std::task::Poll::Pending
    #[cfg(windows)]
    pub(crate) fn operate_blocking(&mut self) -> io::Result<usize> {
        use std::task::Poll;

        let optr = self.extra_mut().optr();
        let op = self.as_pinned_op();
        let res = unsafe { op.operate(optr.cast()) };
        match res {
            Poll::Pending => unreachable!("this operation is not overlapped"),
            Poll::Ready(res) => res,
        }
    }
}

impl<T: ?Sized> std::fmt::Debug for Key<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Key({})", self.user_data())
    }
}
