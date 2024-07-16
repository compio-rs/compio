use std::{ffi::CString, io};

use compio_buf::{IntoInner, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};
use socket2::SockAddr;

use super::{
    buffer_pool::{BorrowedBuffer, BufferPool},
    *,
};
pub use crate::unix::op::*;
use crate::{SharedFd, TakeBuffer};

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
                    }
                }
            }
        }

        impl<$($ty: $trait),*> poll::OpCode for $name<$($ty),*> {
            fn pre_submit(self: std::pin::Pin<&mut Self>) -> std::io::Result<crate::Decision> {
                unsafe { self.map_unchecked_mut(|x| x.inner.poll() ) }.pre_submit()
            }

            fn on_event(
                self: std::pin::Pin<&mut Self>,
                event: &polling::Event,
            ) -> std::task::Poll<std::io::Result<usize>> {
                unsafe { self.map_unchecked_mut(|x| x.inner.poll() ) }.on_event(event)
            }
        }

        impl<$($ty: $trait),*> iour::OpCode for $name<$($ty),*> {
            fn create_entry(self: std::pin::Pin<&mut Self>) -> OpEntry {
                unsafe { self.map_unchecked_mut(|x| x.inner.iour() ) }.create_entry()
            }
        }
    };
}

macro_rules! buffer_pool_op {
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

            impl<$($ty: $trait),*> TakeBuffer<usize> for $name <$($ty),*> {
                type BufferPool = BufferPool;
                type Buffer<'a> = BorrowedBuffer<'a>;

                fn take_buffer(
                    self,
                    buffer_pool: &Self::BufferPool,
                    result: io::Result<usize>,
                    flags: u32,
                ) -> io::Result<Self::Buffer<'_>> {
                    match self.inner {
                        [< $name Inner >]::Poll(inner) => {
                            Ok(BorrowedBuffer::new_poll(inner.take_buffer(buffer_pool.as_poll(), result, flags)?))
                        }
                        [< $name Inner >]::IoUring(inner) => {
                            Ok(BorrowedBuffer::new_io_uring(inner.take_buffer(buffer_pool.as_io_uring(), result, flags)?))
                        }
                    }
                }
            }

            impl<$($ty: $trait),*> $name <$($ty),*> {
                #[doc = concat!("Create a new `", stringify!($name), "`.")]
                pub fn new(buffer_pool: &BufferPool, $($arg: $arg_t),*) -> io::Result<Self> {
                    let this = match DriverType::current() {
                        DriverType::Poll => Self {
                            inner: [< $name Inner >]::Poll(poll::$name::new(buffer_pool.as_poll(), $($arg),*)?),
                        },
                        DriverType::IoUring => Self {
                            inner: [< $name Inner >]::IoUring(iour::$name::new(buffer_pool.as_io_uring(), $($arg),*)?),
                        },
                    };

                    Ok(this)
                }
            }
        }

        impl<$($ty: $trait),*> poll::OpCode for $name<$($ty),*> {
            fn pre_submit(self: std::pin::Pin<&mut Self>) -> std::io::Result<crate::Decision> {
                unsafe { self.map_unchecked_mut(|x| x.inner.poll() ) }.pre_submit()
            }

            fn on_event(
                self: std::pin::Pin<&mut Self>,
                event: &polling::Event,
            ) -> std::task::Poll<std::io::Result<usize>> {
                unsafe { self.map_unchecked_mut(|x| x.inner.poll() ) }.on_event(event)
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

op!(<T: IoBufMut, S: AsRawFd> RecvFrom(fd: SharedFd<S>, buffer: T));
op!(<T: IoBuf, S: AsRawFd> SendTo(fd: SharedFd<S>, buffer: T, addr: SockAddr));
op!(<T: IoVectoredBufMut, S: AsRawFd> RecvFromVectored(fd: SharedFd<S>, buffer: T));
op!(<T: IoVectoredBuf, S: AsRawFd> SendToVectored(fd: SharedFd<S>, buffer: T, addr: SockAddr));
op!(<S: AsRawFd> FileStat(fd: SharedFd<S>));
op!(<> PathStat(path: CString, follow_symlink: bool));

buffer_pool_op!(<S: AsRawFd> RecvBufferPool(fd: SharedFd<S>, len: u32));
buffer_pool_op!(<S: AsRawFd> ReadAtBufferPool(fd: SharedFd<S>, offset: u64, len: u32));
