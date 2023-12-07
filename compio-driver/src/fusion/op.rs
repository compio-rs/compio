use std::ffi::CString;

use compio_buf::{IntoInner, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};
use socket2::SockAddr;

use super::*;
pub use crate::unix::op::*;

macro_rules! op {
    (<$($ty:ident: $trait:ident),* $(,)?> $name:ident( $($arg:ident: $arg_t:ident),* $(,)? )) => {
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
            fn create_entry(self: std::pin::Pin<&mut Self>) -> io_uring::squeue::Entry {
                unsafe { self.map_unchecked_mut(|x| x.inner.iour() ) }.create_entry()
            }
        }
    };
}

#[rustfmt::skip]
mod iour { pub use crate::sys::iour::{op::*, OpCode}; }
#[rustfmt::skip]
mod poll { pub use crate::sys::poll::{op::*, OpCode}; }

op!(<T: IoBufMut> RecvFrom(fd: RawFd, buffer: T));
op!(<T: IoBuf> SendTo(fd: RawFd, buffer: T, addr: SockAddr));
op!(<T: IoVectoredBufMut> RecvFromVectored(fd: RawFd, buffer: T));
op!(<T: IoVectoredBuf> SendToVectored(fd: RawFd, buffer: T, addr: SockAddr));
op!(<> FileStat(fd: RawFd));
op!(<> PathStat(path: CString));
