use std::ffi::CString;

use compio_buf::{IntoInner, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};
use socket2::SockAddr;

use super::*;
pub use crate::unix::op::*;

macro_rules! op {
    (<$($ty:ident: $trait:ident),* $(,)?> $name:ident( $($arg:ident: $arg_t:ty),* $(,)? )) => {
        ::paste::paste!{
            enum [< $name Inner >] <$($ty: $trait),*> {
                Poll(poll::$name<$($ty),*>),
                IoUring(iour::$name<$($ty),*>),
            }

            impl<$($ty: $trait),*> [< $name Inner >]<$($ty),*> {
                fn poll(&mut self) -> &mut poll::$name<$($ty),*> {
                    debug_assert!(DriverType::current() == DriverType::Poll);

                    match self {
                        Self::Poll(ref mut op) => op,
                        Self::IoUring(_) => unreachable!("Current driver is not `io-uring`"),
                    }
                }

                fn iour(&mut self) -> &mut iour::$name<$($ty),*> {
                    debug_assert!(DriverType::current() == DriverType::IoUring);

                    match self {
                        Self::IoUring(ref mut op) => op,
                        Self::Poll(_) => unreachable!("Current driver is not `polling`"),
                    }
                }
            }

            #[doc = concat!("A fused `", stringify!($name), "` operation")]
            pub struct $name <$($ty: $trait),*> {
                inner: [< $name Inner >] <$($ty),*>
            }

            impl<$($ty: $trait),*> IntoInner for $name <$($ty),*> {
                type Inner = <poll::$name<$($ty),*> as IntoInner>::Inner;

                fn into_inner(self) -> Self::Inner {
                    match self.inner {
                        [< $name Inner >]::Poll(op) => op.into_inner(),
                        [< $name Inner >]::IoUring(op) => op.into_inner(),
                    }
                }
            }

            impl<$($ty: $trait),*> $name <$($ty),*> {
                #[doc = concat!("Create a new `", stringify!($name), "`.")]
                pub fn new($($arg: $arg_t),*) -> Self {
                    match DriverType::current() {
                        DriverType::Poll => Self {
                            inner: [< $name Inner >]::Poll(poll::$name::new($($arg),*)),
                        },
                        DriverType::IoUring => Self {
                            inner: [< $name Inner >]::IoUring(iour::$name::new($($arg),*)),
                        },
                        _ => unreachable!("Fuse driver will only be enabled on linux"),
                    }
                }
            }
        }

        impl<$($ty: $trait),*> poll::OpCode for $name<$($ty),*> {
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

        impl<$($ty: $trait),*> iour::OpCode for $name<$($ty),*> {
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

op!(<T: IoBufMut, S: AsFd> RecvFrom(fd: S, buffer: T));
op!(<T: IoBuf, S: AsFd> SendTo(fd: S, buffer: T, addr: SockAddr));
op!(<T: IoVectoredBufMut, S: AsFd> RecvFromVectored(fd: S, buffer: T));
op!(<T: IoVectoredBuf, S: AsFd> SendToVectored(fd: S, buffer: T, addr: SockAddr));
op!(<S: AsFd> FileStat(fd: S));
op!(<> PathStat(path: CString, follow_symlink: bool));

#[cfg(io_uring)]
macro_rules! mop {
    (<$($ty:ident: $trait:ident),* $(,)?> $name:ident( $($arg:ident: $arg_t:ty),* $(,)? )) => {
        ::paste::paste!{
            enum [< $name Inner >] <$($ty: $trait),*> {
                Poll(crate::op::managed::$name<$($ty),*>),
                IoUring(iour::$name<$($ty),*>),
            }

            impl<$($ty: $trait),*> [< $name Inner >]<$($ty),*> {
                fn poll(&mut self) -> &mut crate::op::managed::$name<$($ty),*> {
                    debug_assert!(DriverType::current() == DriverType::Poll);

                    match self {
                        Self::Poll(ref mut op) => op,
                        Self::IoUring(_) => unreachable!("Current driver is not `io-uring`"),
                    }
                }

                fn iour(&mut self) -> &mut iour::$name<$($ty),*> {
                    debug_assert!(DriverType::current() == DriverType::IoUring);

                    match self {
                        Self::IoUring(ref mut op) => op,
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
                    Ok(match DriverType::current() {
                        DriverType::Poll => Self {
                            inner: [< $name Inner >]::Poll(crate::op::managed::$name::new($($arg),*)?),
                        },
                        DriverType::IoUring => Self {
                            inner: [< $name Inner >]::IoUring(iour::$name::new($($arg),*)?),
                        },
                        _ => unreachable!("Fuse driver will only be enabled on linux"),
                    })
                }
            }

            impl<$($ty: $trait),*> crate::TakeBuffer for $name<$($ty),*> {
                type BufferPool = crate::BufferPool;
                type Buffer<'a> = crate::BorrowedBuffer<'a>;

                fn take_buffer(
                    self,
                    buffer_pool: &Self::BufferPool,
                    result: io::Result<usize>,
                    flags: u32,
                ) -> io::Result<Self::Buffer<'_>> {
                    match self.inner {
                        [< $name Inner >]::Poll(inner) => {
                            Ok(inner.take_buffer(buffer_pool, result, flags)?)
                        }
                        [< $name Inner >]::IoUring(inner) => {
                            Ok(inner.take_buffer(buffer_pool, result, flags)?)
                        }
                    }
                }
            }
        }

        impl<$($ty: $trait),*> poll::OpCode for $name<$($ty),*> {
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

        impl<$($ty: $trait),*> iour::OpCode for $name<$($ty),*> {
            fn create_entry(self: std::pin::Pin<&mut Self>) -> OpEntry {
                unsafe { self.map_unchecked_mut(|x| x.inner.iour() ) }.create_entry()
            }
        }
    };
}

#[cfg(io_uring)]
mop!(<S: AsFd> ReadManagedAt(fd: S, offset: u64, pool: &BufferPool, len: usize));
#[cfg(io_uring)]
mop!(<S: AsFd> RecvManaged(fd: S, pool: &BufferPool, len: usize));
