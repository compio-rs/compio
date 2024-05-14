use std::{
    io,
    marker::PhantomData,
    ops::{Deref, DerefMut},
    pin::Pin,
    task::Waker,
};

use compio_buf::BufResult;

use crate::{OpCode, Overlapped, PushEntry, RawFd};

#[repr(C)]
pub struct RawOp<T: ?Sized> {
    header: Overlapped,
    // The two flags here are manual reference counting. The driver holds the strong ref until it
    // completes; the runtime holds the strong ref until the future is dropped.
    cancelled: bool,
    upcast_fn: unsafe fn(usize) -> *mut RawOp<dyn OpCode>,
    result: PushEntry<Option<Waker>, io::Result<usize>>,
    op: T,
}

impl<T: ?Sized> RawOp<T> {
    pub fn as_op_pin(&mut self) -> Pin<&mut T> {
        unsafe { Pin::new_unchecked(&mut self.op) }
    }

    #[cfg(windows)]
    pub fn as_mut_ptr(&mut self) -> *mut Overlapped {
        &mut self.header
    }

    pub fn set_cancelled(&mut self) -> bool {
        self.cancelled = true;
        self.has_result()
    }

    pub fn set_result(&mut self, res: io::Result<usize>) -> bool {
        if let PushEntry::Pending(Some(w)) =
            std::mem::replace(&mut self.result, PushEntry::Ready(res))
        {
            w.wake();
        }
        self.cancelled
    }

    pub fn has_result(&self) -> bool {
        self.result.is_ready()
    }

    pub fn set_waker(&mut self, waker: Waker) {
        if let PushEntry::Pending(w) = &mut self.result {
            *w = Some(waker)
        }
    }

    pub fn into_inner(self) -> BufResult<usize, T>
    where
        T: Sized,
    {
        BufResult(self.result.take_ready().unwrap(), self.op)
    }
}

#[cfg(windows)]
impl<T: OpCode + ?Sized> RawOp<T> {
    pub fn operate_blocking(&mut self) -> io::Result<usize> {
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

unsafe fn upcast<T: OpCode>(user_data: usize) -> *mut RawOp<dyn OpCode> {
    user_data as *mut RawOp<T> as *mut RawOp<dyn OpCode>
}

/// A typed wrapper for key of Ops submitted into driver
#[derive(PartialEq, Eq, Hash)]
pub struct Key<T> {
    user_data: usize,
    _p: PhantomData<Box<T>>,
}

impl<T> Unpin for Key<T> {}

impl<T: OpCode + 'static> Key<T> {
    /// Create [`RawOp`] and get the [`Key`] to it.
    pub fn new(driver: RawFd, op: T) -> Self {
        let header = Overlapped::new(driver);
        let raw_op = Box::new(RawOp {
            header,
            cancelled: false,
            upcast_fn: upcast::<T>,
            result: PushEntry::Pending(None),
            op,
        });
        unsafe { Self::new_unchecked(Box::into_raw(raw_op) as _) }
    }
}

impl<T> Key<T> {
    /// Create a new `Key` with the given user data.
    ///
    /// # Safety
    ///
    /// Caller needs to ensure that `T` does correspond to `user_data` in driver
    /// this `Key` is created with.
    pub unsafe fn new_unchecked(user_data: usize) -> Self {
        Self {
            user_data,
            _p: PhantomData,
        }
    }

    /// Get the unique user-defined data.
    pub const fn user_data(&self) -> usize {
        self.user_data
    }

    /// Get the inner result if it is completed.
    pub fn into_inner(self) -> BufResult<usize, T> {
        unsafe { Box::from_raw(self.user_data as *mut RawOp<T>) }.into_inner()
    }
}

impl Key<()> {
    pub(crate) unsafe fn drop_in_place(user_data: usize) {
        let op = &*(user_data as *const RawOp<()>);
        let ptr = (op.upcast_fn)(user_data);
        let _ = Box::from_raw(ptr);
    }

    pub(crate) unsafe fn upcast<'a>(user_data: usize) -> &'a mut RawOp<dyn OpCode> {
        let op = &*(user_data as *const RawOp<()>);
        &mut *(op.upcast_fn)(user_data)
    }
}

impl<T> Deref for Key<T> {
    type Target = RawOp<T>;

    fn deref(&self) -> &Self::Target {
        unsafe { &*(self.user_data as *const RawOp<T>) }
    }
}

impl<T> DerefMut for Key<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *(self.user_data as *mut RawOp<T>) }
    }
}

impl<T> std::fmt::Debug for Key<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Key({})", self.user_data)
    }
}
