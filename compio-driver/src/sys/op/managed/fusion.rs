use compio_buf::*;
use rustix::net::RecvFlags;
use socket2::SockAddr;

use super::{fallback, iour};
use crate::{BufferPool, BufferRef, IourOpCode, OpEntry, OpType, PollOpCode, sys::pal::*};

macro_rules! mop {
    (<$($ty:ident: $trait:ident),* $(,)?> $name:ident( $($arg:ident: $arg_t:ty),* $(,)? ) with $pool:ident) => {
        mop!(<$($ty: $trait),*> $name( $($arg: $arg_t),* ) with $pool; crate::BufferRef);
    };
    (<$($ty:ident: $trait:ident),* $(,)?> $name:ident( $($arg:ident: $arg_t:ty),* $(,)? ) with $pool:ident; $inner:ty) => {
        ::paste::paste!{
            enum [< $name Inner >] <$($ty: $trait),*> {
                Poll(fallback::$name<$($ty),*>),
                IoUring(iour::$name<$($ty),*>),
            }

            impl<$($ty: $trait),*> [< $name Inner >]<$($ty),*> {
                fn poll(&mut self) -> &mut fallback::$name<$($ty),*> {
                    match self {
                        Self::Poll(op) => op,
                        Self::IoUring(_) => unreachable!("Current driver is not `io-uring`"),
                    }
                }

                fn iour(&mut self) -> &mut iour::$name<$($ty),*> {
                    match self {
                        Self::IoUring(op) => op,
                        Self::Poll(_) => unreachable!("Current driver is not `polling`"),
                    }
                }
            }

            #[doc = concat!("A fused `", stringify!($name), "` operation")]
            pub struct $name <$($ty: $trait),*> {
                inner: [< $name Inner >] <$($ty),*>
            }

            impl<$($ty: $trait),*> $name <$($ty),*> {
                #[doc = concat!("Create a new `", stringify!($name), "`.")]
                pub fn new($($arg: $arg_t),*) -> std::io::Result<Self> {
                    Ok(if $pool.is_io_uring()? {
                        Self {
                            inner: [< $name Inner >]::IoUring(iour::$name::new($($arg),*)?),
                        }
                    } else {
                        Self {
                            inner: [< $name Inner >]::Poll(fallback::$name::new($($arg),*)?),
                        }
                    })
                }
            }

            impl <$($ty: $trait),*> crate::TakeBuffer for $name <$($ty),*> {
                type Buffer = $inner;

                fn take_buffer(self) -> Option<$inner> {
                    match self.inner {
                        [< $name Inner >]::IoUring(op) => op.take_buffer().map(Into::into),
                        [< $name Inner >]::Poll(op) => op.take_buffer().map(Into::into),
                    }
                }
            }

            unsafe impl<$($ty: $trait),*> PollOpCode for $name<$($ty),*> {
                type Control = <fallback::$name<$($ty),*> as PollOpCode>::Control;

                unsafe fn init(&mut self, ctrl: &mut Self::Control) {
                    unsafe { self.inner.poll().init(ctrl) }
                }

                fn pre_submit(&mut self, control: &mut Self::Control) -> std::io::Result<crate::Decision> {
                    self.inner.poll().pre_submit(control)
                }

                fn op_type(&mut self, control: &mut Self::Control) -> Option<OpType> {
                    self.inner.poll().op_type(control)
                }

                fn operate(
                    &mut self, control: &mut Self::Control,
                ) -> std::task::Poll<std::io::Result<usize>> {
                    self.inner.poll().operate(control)
                }
            }

            unsafe impl<$($ty: $trait),*> IourOpCode for $name<$($ty),*> {
                type Control = <iour::$name<$($ty),*> as IourOpCode>::Control;

                unsafe fn init(&mut self, ctrl: &mut Self::Control) {
                    unsafe { self.inner.iour().init(ctrl) }
                }

                fn create_entry(&mut self, control: &mut Self::Control) -> OpEntry {
                    self.inner.iour().create_entry(control)
                }

                fn create_entry_fallback(&mut self, control: &mut Self::Control) -> OpEntry {
                    self.inner.iour().create_entry_fallback(control)
                }

                fn call_blocking(&mut self, control: &mut Self::Control) -> std::io::Result<usize> {
                    self.inner.iour().call_blocking(control)
                }

                unsafe fn set_result(&mut self, control: &mut Self::Control, result: &std::io::Result<usize>, extra: &crate::Extra) {
                    unsafe { self.inner.iour().set_result(control, result, extra) }
                }

                unsafe fn push_multishot(&mut self, control: &mut Self::Control, result: std::io::Result<usize>, extra: crate::Extra) {
                    unsafe { self.inner.iour().push_multishot(control, result, extra) }
                }

                fn pop_multishot(&mut self, control: &mut Self::Control) -> Option<BufResult<usize, crate::Extra>> {
                    self.inner.iour().pop_multishot(control)
                }
            }
        }
    };
}

mop!(<S: AsFd> ReadManagedAt(fd: S, offset: u64, pool: &BufferPool, len: usize) with pool);
mop!(<S: AsFd> ReadManaged(fd: S, pool: &BufferPool, len: usize) with pool);
mop!(<S: AsFd> RecvManaged(fd: S, pool: &BufferPool, len: usize, flags: RecvFlags) with pool);
mop!(<S: AsFd> RecvFromManaged(fd: S, pool: &BufferPool, len: usize, flags: RecvFlags) with pool; (BufferRef, Option<SockAddr>));
mop!(<C: IoBufMut, S: AsFd> RecvMsgManaged(fd: S, pool: &BufferPool, len: usize, control: C, flags: RecvFlags) with pool; ((BufferRef, C), Option<SockAddr>, usize));
mop!(<S: AsFd> ReadMultiAt(fd: S, offset: u64, pool: &BufferPool, len: usize) with pool);
mop!(<S: AsFd> ReadMulti(fd: S, pool: &BufferPool, len: usize) with pool);
mop!(<S: AsFd> RecvMulti(fd: S, pool: &BufferPool, len: usize, flags: RecvFlags) with pool);
mop!(<S: AsFd> RecvFromMulti(fd: S, pool: &BufferPool, flags: RecvFlags) with pool; RecvFromMultiResult);
mop!(<S: AsFd> RecvMsgMulti(fd: S, pool: &BufferPool, control_len: usize, flags: RecvFlags) with pool; RecvMsgMultiResult);

enum RecvFromMultiResultInner {
    Poll(fallback::RecvFromMultiResult),
    IoUring(iour::RecvFromMultiResult),
}

/// Result of [`RecvFromMulti`].
pub struct RecvFromMultiResult {
    inner: RecvFromMultiResultInner,
}

impl From<fallback::RecvFromMultiResult> for RecvFromMultiResult {
    fn from(result: fallback::RecvFromMultiResult) -> Self {
        Self {
            inner: RecvFromMultiResultInner::Poll(result),
        }
    }
}

impl From<iour::RecvFromMultiResult> for RecvFromMultiResult {
    fn from(result: iour::RecvFromMultiResult) -> Self {
        Self {
            inner: RecvFromMultiResultInner::IoUring(result),
        }
    }
}

impl RecvFromMultiResult {
    /// Create [`RecvFromMultiResult`] from a buffer received from
    /// [`RecvFromMulti`]. It should be used for io-uring only.
    ///
    /// # Safety
    ///
    /// The buffer must be received from [`RecvFromMulti`] or have the same
    /// format as the buffer received from [`RecvFromMulti`].
    pub unsafe fn new(buffer: BufferRef) -> Self {
        Self {
            inner: RecvFromMultiResultInner::IoUring(unsafe {
                iour::RecvFromMultiResult::new(buffer)
            }),
        }
    }

    /// Get the payload data.
    pub fn data(&self) -> &[u8] {
        match &self.inner {
            RecvFromMultiResultInner::Poll(result) => result.data(),
            RecvFromMultiResultInner::IoUring(result) => result.data(),
        }
    }

    /// Get the source address if applicable.
    pub fn addr(&self) -> Option<SockAddr> {
        match &self.inner {
            RecvFromMultiResultInner::Poll(result) => result.addr(),
            RecvFromMultiResultInner::IoUring(result) => result.addr(),
        }
    }
}

impl IntoInner for RecvFromMultiResult {
    type Inner = BufferRef;

    fn into_inner(self) -> Self::Inner {
        match self.inner {
            RecvFromMultiResultInner::Poll(result) => result.into_inner(),
            RecvFromMultiResultInner::IoUring(result) => result.into_inner(),
        }
    }
}

enum RecvMsgMultiResultInner {
    Poll(fallback::RecvMsgMultiResult),
    IoUring(iour::RecvMsgMultiResult),
}

/// Result of [`RecvMsgMulti`].
pub struct RecvMsgMultiResult {
    inner: RecvMsgMultiResultInner,
}

impl From<fallback::RecvMsgMultiResult> for RecvMsgMultiResult {
    fn from(result: fallback::RecvMsgMultiResult) -> Self {
        Self {
            inner: RecvMsgMultiResultInner::Poll(result),
        }
    }
}

impl From<iour::RecvMsgMultiResult> for RecvMsgMultiResult {
    fn from(result: iour::RecvMsgMultiResult) -> Self {
        Self {
            inner: RecvMsgMultiResultInner::IoUring(result),
        }
    }
}

impl RecvMsgMultiResult {
    /// Create [`RecvMsgMultiResult`] from a buffer received from
    /// [`RecvMsgMulti`]. It should be used for io-uring only.
    ///
    /// # Safety
    ///
    /// The buffer must be received from [`RecvMsgMulti`] or have the same
    /// format as the buffer received from [`RecvMsgMulti`].
    pub unsafe fn new(buffer: BufferRef, clen: usize) -> Self {
        Self {
            inner: RecvMsgMultiResultInner::IoUring(unsafe {
                iour::RecvMsgMultiResult::new(buffer, clen)
            }),
        }
    }

    /// Get the payload data.
    pub fn data(&self) -> &[u8] {
        match &self.inner {
            RecvMsgMultiResultInner::Poll(result) => result.data(),
            RecvMsgMultiResultInner::IoUring(result) => result.data(),
        }
    }

    /// Get the ancillary data.
    pub fn ancillary(&self) -> &[u8] {
        match &self.inner {
            RecvMsgMultiResultInner::Poll(result) => result.ancillary(),
            RecvMsgMultiResultInner::IoUring(result) => result.ancillary(),
        }
    }

    /// Get the source address if applicable.
    pub fn addr(&self) -> Option<SockAddr> {
        match &self.inner {
            RecvMsgMultiResultInner::Poll(result) => result.addr(),
            RecvMsgMultiResultInner::IoUring(result) => result.addr(),
        }
    }
}

impl IntoInner for RecvMsgMultiResult {
    type Inner = BufferRef;

    fn into_inner(self) -> Self::Inner {
        match self.inner {
            RecvMsgMultiResultInner::Poll(result) => result.into_inner(),
            RecvMsgMultiResultInner::IoUring(result) => result.into_inner(),
        }
    }
}
