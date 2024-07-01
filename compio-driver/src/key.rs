use std::{io, marker::PhantomData, mem::MaybeUninit, pin::Pin, task::Waker};

use compio_buf::BufResult;

use crate::{OpCode, Overlapped, PushEntry, RawFd};

/// An operation with other needed information. It should be allocated on the
/// heap. The pointer to this struct is used as `user_data`, and on Windows, it
/// is used as the pointer to `OVERLAPPED`.
///
/// `*const RawOp<dyn OpCode>` can be obtained from any `Key<T: OpCode>` by
/// first casting `Key::user_data` to `*const RawOp<()>`, then upcasted with
/// `upcast_fn`. It is done in [`Key::as_op_pin`].
#[repr(C)]
pub(crate) struct RawOp<T: ?Sized> {
    header: Overlapped,
    // The cancelled flag and the result here are manual reference counting. The driver holds the
    // strong ref until it completes; the runtime holds the strong ref until the future is
    // dropped.
    cancelled: bool,
    // The metadata in `*mut RawOp<dyn OpCode>`
    metadata: usize,
    result: PushEntry<Option<Waker>, io::Result<usize>>,
    flags: u32,
    op: T,
}

#[repr(C)]
union OpCodePtrRepr {
    ptr: *mut RawOp<dyn OpCode>,
    components: OpCodePtrComponents,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct OpCodePtrComponents {
    data_pointer: *mut (),
    metadata: usize,
}

fn opcode_metadata<T: OpCode + 'static>() -> usize {
    let mut op = MaybeUninit::<RawOp<T>>::uninit();
    // SAFETY: same as `core::ptr::metadata`.
    unsafe {
        OpCodePtrRepr {
            ptr: op.as_mut_ptr(),
        }
        .components
        .metadata
    }
}

const unsafe fn opcode_dyn_mut(ptr: *mut (), metadata: usize) -> *mut RawOp<dyn OpCode> {
    OpCodePtrRepr {
        components: OpCodePtrComponents {
            data_pointer: ptr,
            metadata,
        },
    }
    .ptr
}

/// A typed wrapper for key of Ops submitted into driver. It doesn't free the
/// inner on dropping. Instead, the memory is managed by the proactor. The inner
/// is only freed when:
///
/// 1. The op is completed and the future asks the result. `into_inner` will be
///    called by the proactor.
/// 2. The op is completed and the future cancels it. `into_box` will be called
///    by the proactor.
#[derive(PartialEq, Eq, Hash)]
pub struct Key<T: ?Sized> {
    user_data: *mut (),
    _p: PhantomData<Box<RawOp<T>>>,
}

impl<T: ?Sized> Unpin for Key<T> {}

impl<T: OpCode + 'static> Key<T> {
    /// Create [`RawOp`] and get the [`Key`] to it.
    pub(crate) fn new(driver: RawFd, op: T) -> Self {
        let header = Overlapped::new(driver);
        let raw_op = Box::new(RawOp {
            header,
            cancelled: false,
            metadata: opcode_metadata::<T>(),
            result: PushEntry::Pending(None),
            flags: 0,
            op,
        });
        unsafe { Self::new_unchecked(Box::into_raw(raw_op) as _) }
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
            user_data: user_data as _,
            _p: PhantomData,
        }
    }

    /// Get the unique user-defined data.
    pub fn user_data(&self) -> usize {
        self.user_data as _
    }

    fn as_opaque(&self) -> &RawOp<()> {
        // SAFETY: user_data is unique and RawOp is repr(C).
        unsafe { &*(self.user_data as *const RawOp<()>) }
    }

    fn as_opaque_mut(&mut self) -> &mut RawOp<()> {
        // SAFETY: see `as_opaque`.
        unsafe { &mut *(self.user_data as *mut RawOp<()>) }
    }

    fn as_dyn_mut_ptr(&mut self) -> *mut RawOp<dyn OpCode> {
        let user_data = self.user_data;
        let this = self.as_opaque_mut();
        // SAFETY: metadata from `Key::new`.
        unsafe { opcode_dyn_mut(user_data, this.metadata) }
    }

    /// A pointer to OVERLAPPED.
    #[cfg(windows)]
    pub(crate) fn as_mut_ptr(&mut self) -> *mut Overlapped {
        &mut self.as_opaque_mut().header
    }

    /// Cancel the op, decrease the ref count. The return value indicates if the
    /// op is completed. If so, the op should be dropped because it is
    /// useless.
    pub(crate) fn set_cancelled(&mut self) -> bool {
        self.as_opaque_mut().cancelled = true;
        self.has_result()
    }

    /// Complete the op, decrease the ref count. Wake the future if a waker is
    /// set. The return value indicates if the op is cancelled. If so, the
    /// op should be dropped because it is useless.
    pub(crate) fn set_result(&mut self, res: io::Result<usize>) -> bool {
        let this = self.as_opaque_mut();
        if let PushEntry::Pending(Some(w)) =
            std::mem::replace(&mut this.result, PushEntry::Ready(res))
        {
            w.wake();
        }
        this.cancelled
    }

    pub(crate) fn set_flags(&mut self, flags: u32) {
        self.as_opaque_mut().flags = flags;
    }

    /// Whether the op is completed.
    pub(crate) fn has_result(&self) -> bool {
        self.as_opaque().result.is_ready()
    }

    /// Set waker of the current future.
    pub(crate) fn set_waker(&mut self, waker: Waker) {
        if let PushEntry::Pending(w) = &mut self.as_opaque_mut().result {
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
        Box::from_raw(self.as_dyn_mut_ptr())
    }
}

impl<T> Key<T> {
    /// Get the inner result if it is completed.
    ///
    /// # Safety
    ///
    /// Call it only when the op is completed, otherwise it is UB.
    pub(crate) unsafe fn into_inner(self) -> BufResult<usize, T> {
        let op = unsafe { Box::from_raw(self.user_data as *mut RawOp<T>) };
        BufResult(op.result.take_ready().unwrap_unchecked(), op.op)
    }

    /// Get the inner result and flags if it is completed.
    ///
    /// # Safety
    ///
    /// Call it only when the op is completed, otherwise it is UB.
    pub(crate) unsafe fn into_inner_flags(self) -> (BufResult<usize, T>, u32) {
        let op = unsafe { Box::from_raw(self.user_data as *mut RawOp<T>) };
        (
            BufResult(op.result.take_ready().unwrap_unchecked(), op.op),
            op.flags,
        )
    }
}

impl<T: OpCode + ?Sized> Key<T> {
    /// Pin the inner op.
    pub(crate) fn as_op_pin(&mut self) -> Pin<&mut dyn OpCode> {
        // SAFETY: the inner won't be moved.
        unsafe {
            let this = &mut *self.as_dyn_mut_ptr();
            Pin::new_unchecked(&mut this.op)
        }
    }

    /// Call [`OpCode::operate`] and assume that it is not an overlapped op,
    /// which means it never returns [`Poll::Pending`].
    ///
    /// [`Poll::Pending`]: std::task::Poll::Pending
    #[cfg(windows)]
    pub(crate) fn operate_blocking(&mut self) -> io::Result<usize> {
        use std::task::Poll;

        let optr = self.as_mut_ptr();
        let op = self.as_op_pin();
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
