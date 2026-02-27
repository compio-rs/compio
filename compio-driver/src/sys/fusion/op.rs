use std::{ffi::CString, hint::unreachable_unchecked};

use compio_buf::{IntoInner, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};
use socket2::SockAddr;

use super::*;
pub use crate::sys::unix_op::*;

macro_rules! op {
    (<$($ty:ident: $trait:ident),* $(,)?> $name:ident( $($arg:ident: $arg_t:ty),* $(,)? )) => {
        ::paste::paste!{
            enum [< $name Inner >] <$($ty: $trait),*> {
                Uninit($($arg_t),*),
                Poll(poll::$name<$($ty),*>),
                IoUring(iour::$name<$($ty),*>),
            }

            impl<$($ty: $trait),*> [< $name Inner >]<$($ty),*> {
                fn poll(&mut self) -> &mut poll::$name<$($ty),*> {
                    match self {
                        Self::Uninit(..) => {
                            unsafe {
                                let Self::Uninit($($arg),*) = std::ptr::read(self) else {
                                    unreachable_unchecked()
                                };
                                std::ptr::write(self, Self::Poll(poll::$name::new($($arg),*)));
                            }
                            self.poll()
                        },
                        Self::Poll(op) => op,
                        Self::IoUring(_) => unreachable!("Current driver is not `polling`"),
                    }
                }

                fn iour(&mut self) -> &mut iour::$name<$($ty),*> {
                    match self {
                        Self::Uninit(..) => {
                            unsafe {
                                let Self::Uninit($($arg),*) = std::ptr::read(self) else {
                                    unreachable_unchecked()
                                };
                                std::ptr::write(self, Self::IoUring(iour::$name::new($($arg),*)));
                            }
                            self.iour()
                        },
                        Self::IoUring(op) => op,
                        Self::Poll(_) => unreachable!("Current driver is not `io-uring`"),
                    }
                }
            }

            #[doc = concat!("A fused `", stringify!($name), "` operation")]
            pub struct $name <$($ty: $trait),*> {
                inner: [< $name Inner >] <$($ty),*>
            }

            impl<$($ty: $trait),*> IntoInner for $name <$($ty),*> {
                type Inner = <poll::$name<$($ty),*> as IntoInner>::Inner;

                fn into_inner(mut self) -> Self::Inner {
                    match self.inner {
                        [< $name Inner >]::Uninit(..) => {
                            self.inner.poll();
                            self.into_inner()
                        },
                        [< $name Inner >]::Poll(op) => op.into_inner(),
                        [< $name Inner >]::IoUring(op) => op.into_inner(),
                    }
                }
            }

            impl<$($ty: $trait),*> $name <$($ty),*> {
                #[doc = concat!("Create a new `", stringify!($name), "`.")]
                pub fn new($($arg: $arg_t),*) -> Self {
                    Self { inner: [< $name Inner >]::Uninit($($arg),*) }
                }
            }
        }

        unsafe impl<$($ty: $trait),*> poll::OpCode for $name<$($ty),*> {
            fn pre_submit(self: std::pin::Pin<&mut Self>) -> std::io::Result<crate::Decision> {
                unsafe { self.map_unchecked_mut(|x| x.inner.poll() ) }.pre_submit()
            }

            fn op_type(self: std::pin::Pin<&mut Self>) -> Option<OpType> {
                unsafe { self.map_unchecked_mut(|x| x.inner.poll() ) }.op_type()
            }

            fn operate(
                self: std::pin::Pin<&mut Self>,
            ) -> std::task::Poll<std::io::Result<usize>> {
                unsafe { self.map_unchecked_mut(|x| x.inner.poll() ) }.operate()
            }
        }

        unsafe impl<$($ty: $trait),*> iour::OpCode for $name<$($ty),*> {
            fn create_entry(self: std::pin::Pin<&mut Self>) -> OpEntry {
                unsafe { self.map_unchecked_mut(|x| x.inner.iour() ) }.create_entry()
            }
        }
    };
}

#[rustfmt::skip]
mod iour { pub use crate::sys::iour::{op::*, OpCode}; }
#[rustfmt::skip]
mod poll { pub use crate::sys::poll::{op::*, OpCode}; }

op!(<T: IoBufMut, S: AsFd> RecvFrom(fd: S, buffer: T, flags: i32));
op!(<T: IoBuf, S: AsFd> SendTo(fd: S, buffer: T, addr: SockAddr, flags: i32));
op!(<T: IoVectoredBufMut, S: AsFd> RecvFromVectored(fd: S, buffer: T, flags: i32));
op!(<T: IoVectoredBuf, S: AsFd> SendToVectored(fd: S, buffer: T, addr: SockAddr, flags: i32));
op!(<S: AsFd> FileStat(fd: S));
op!(<S: AsFd> PathStat(dirfd: S, path: CString, follow_symlink: bool));

macro_rules! mop {
    (<$($ty:ident: $trait:ident),* $(,)?> $name:ident( $($arg:ident: $arg_t:ty),* $(,)? ) with $pool:ident) => {
        mop!{ < $($ty: $trait),* > $name ( $( $arg: $arg_t ),* ) with $pool, buffer: crate::BorrowedBuffer<'a> }
    };
    (<$($ty:ident: $trait:ident),* $(,)?> $name:ident( $($arg:ident: $arg_t:ty),* $(,)? ) with $pool:ident, buffer: $buffer:ty) => {
        ::paste::paste!{
            enum [< $name Inner >] <$($ty: $trait),*> {
                Poll(crate::op::managed::$name<$($ty),*>),
                IoUring(iour::$name<$($ty),*>),
            }

            impl<$($ty: $trait),*> [< $name Inner >]<$($ty),*> {
                fn poll(&mut self) -> &mut crate::op::managed::$name<$($ty),*> {
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
                    Ok(if $pool.is_io_uring() {
                        Self {
                            inner: [< $name Inner >]::IoUring(iour::$name::new($($arg),*)?),
                        }
                    } else {
                        Self {
                            inner: [< $name Inner >]::Poll(crate::op::managed::$name::new($($arg),*)?),
                        }
                    })
                }
            }

            impl<$($ty: $trait),*> crate::TakeBuffer for $name<$($ty),*> {
                type BufferPool = crate::BufferPool;
                type Buffer<'a> = $buffer;

                fn take_buffer(
                    self,
                    buffer_pool: &Self::BufferPool,
                    result: io::Result<usize>,
                    buffer_id: u16,
                ) -> io::Result<Self::Buffer<'_>> {
                    match self.inner {
                        [< $name Inner >]::Poll(inner) => {
                            Ok(inner.take_buffer(buffer_pool, result, buffer_id)?)
                        }
                        [< $name Inner >]::IoUring(inner) => {
                            Ok(inner.take_buffer(buffer_pool, result, buffer_id)?)
                        }
                    }
                }
            }
        }

        unsafe impl<$($ty: $trait),*> poll::OpCode for $name<$($ty),*> {
            fn pre_submit(self: std::pin::Pin<&mut Self>) -> std::io::Result<crate::Decision> {
                unsafe { self.map_unchecked_mut(|x| x.inner.poll() ) }.pre_submit()
            }

            fn op_type(self: std::pin::Pin<&mut Self>) -> Option<OpType> {
                unsafe { self.map_unchecked_mut(|x| x.inner.poll() ) }.op_type()
            }

            fn operate(
                self: std::pin::Pin<&mut Self>,
            ) -> std::task::Poll<std::io::Result<usize>> {
                unsafe { self.map_unchecked_mut(|x| x.inner.poll() ) }.operate()
            }
        }

        unsafe impl<$($ty: $trait),*> iour::OpCode for $name<$($ty),*> {
            fn create_entry(self: std::pin::Pin<&mut Self>) -> OpEntry {
                unsafe { self.map_unchecked_mut(|x| x.inner.iour() ) }.create_entry()
            }
        }
    };
}

mop!(<S: AsFd> ReadManagedAt(fd: S, offset: u64, pool: &BufferPool, len: usize) with pool);
mop!(<S: AsFd> ReadManaged(fd: S, pool: &BufferPool, len: usize) with pool);
mop!(<S: AsFd> RecvManaged(fd: S, pool: &BufferPool, len: usize, flags: i32) with pool);
mop!(<S: AsFd> RecvFromManaged(fd: S, pool: &BufferPool, len: usize, flags: i32) with pool, buffer: (crate::BorrowedBuffer<'a>, SockAddr));
mop!(<S: AsFd> ReadMulti(fd:S, pool: &BufferPool, len: usize) with pool);
