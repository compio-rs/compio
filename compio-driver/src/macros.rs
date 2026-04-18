#[cfg(windows)]
#[macro_export]
#[doc(hidden)]
macro_rules! syscall {
    (BOOL, $e:expr) => {
        $crate::syscall!($e, == 0)
    };
    (SOCKET, $e:expr) => {
        $crate::syscall!($e, != 0)
    };
    (HANDLE, $e:expr) => {
        $crate::syscall!($e, == ::windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE)
    };
    ($e:expr, $op: tt $rhs: expr) => {{
        #[allow(unused_unsafe)]
        let res = unsafe { $e };
        if res $op $rhs {
            Err(::std::io::Error::last_os_error())
        } else {
            Ok(res)
        }
    }};
}

/// Helper macro to execute a system call
#[cfg(unix)]
#[macro_export]
#[doc(hidden)]
macro_rules! syscall {
    (break $e:expr) => {
        loop {
            match $crate::syscall!($e) {
                Ok(fd) => break ::std::task::Poll::Ready(Ok(fd as usize)),
                Err(e) if e.kind() == ::std::io::ErrorKind::WouldBlock || e.raw_os_error() == Some(::libc::EINPROGRESS)
                    => break ::std::task::Poll::Pending,
                Err(e) if e.kind() == ::std::io::ErrorKind::Interrupted => {},
                Err(e) => break ::std::task::Poll::Ready(Err(e)),
            }
        }
    };
    ($e:expr, $f:ident($fd:expr)) => {
        match $crate::syscall!(break $e) {
            ::std::task::Poll::Pending => Ok($crate::sys::Decision::$f($fd)),
            ::std::task::Poll::Ready(Ok(res)) => Ok($crate::sys::Decision::Completed(res)),
            ::std::task::Poll::Ready(Err(e)) => Err(e),
        }
    };
    ($e:expr) => {{
        #[allow(unused_unsafe)]
        let res = unsafe { $e };
        if res == -1 {
            Err(::std::io::Error::last_os_error())
        } else {
            Ok(res as usize)
        }
    }};
}

#[macro_export]
#[doc(hidden)]
macro_rules! impl_raw_fd {
    ($t:ty, $it:ty, $inner:ident) => {
        impl $crate::AsRawFd for $t {
            fn as_raw_fd(&self) -> $crate::RawFd {
                self.$inner.as_raw_fd()
            }
        }
        #[cfg(unix)]
        impl std::os::fd::AsFd for $t {
            fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
                self.$inner.as_fd()
            }
        }
        #[cfg(unix)]
        impl std::os::fd::FromRawFd for $t {
            unsafe fn from_raw_fd(fd: $crate::RawFd) -> Self {
                Self {
                    $inner: unsafe { std::os::fd::FromRawFd::from_raw_fd(fd) },
                }
            }
        }
        impl $crate::ToSharedFd<$it> for $t {
            fn to_shared_fd(&self) -> $crate::SharedFd<$it> {
                self.$inner.to_shared_fd()
            }
        }
    };
    ($t:ty, $it:ty, $inner:ident,file) => {
        $crate::impl_raw_fd!($t, $it, $inner);
        #[cfg(windows)]
        impl std::os::windows::io::FromRawHandle for $t {
            unsafe fn from_raw_handle(handle: std::os::windows::io::RawHandle) -> Self {
                Self {
                    $inner: unsafe { std::os::windows::io::FromRawHandle::from_raw_handle(handle) },
                }
            }
        }
        #[cfg(windows)]
        impl std::os::windows::io::AsHandle for $t {
            fn as_handle(&self) -> std::os::windows::io::BorrowedHandle {
                self.$inner.as_handle()
            }
        }
        #[cfg(windows)]
        impl std::os::windows::io::AsRawHandle for $t {
            fn as_raw_handle(&self) -> std::os::windows::io::RawHandle {
                self.$inner.as_raw_handle()
            }
        }
    };
    ($t:ty, $it:ty, $inner:ident,socket) => {
        $crate::impl_raw_fd!($t, $it, $inner);
        #[cfg(windows)]
        impl std::os::windows::io::FromRawSocket for $t {
            unsafe fn from_raw_socket(sock: std::os::windows::io::RawSocket) -> Self {
                Self {
                    $inner: unsafe { std::os::windows::io::FromRawSocket::from_raw_socket(sock) },
                }
            }
        }
        #[cfg(windows)]
        impl std::os::windows::io::AsSocket for $t {
            fn as_socket(&self) -> std::os::windows::io::BorrowedSocket {
                self.$inner.as_socket()
            }
        }
        #[cfg(windows)]
        impl std::os::windows::io::AsRawSocket for $t {
            fn as_raw_socket(&self) -> std::os::windows::io::RawSocket {
                self.$inner.as_raw_socket()
            }
        }
    };
}

/// Macro that asserts a type *DOES NOT* implement some trait. Shamelessly
/// copied from <https://users.rust-lang.org/t/a-macro-to-assert-that-a-type-does-not-implement-trait-bounds/31179>.
///
/// # Example
///
/// ```rust,ignore
/// assert_not_impl!(u8, From<u16>);
/// ```
#[macro_export]
#[doc(hidden)]
macro_rules! assert_not_impl {
    ($x:ty, $($t:path),+ $(,)*) => {
        const _: fn() -> () = || {
            struct Check<T: ?Sized>(T);
            trait AmbiguousIfImpl<A> { fn some_item() { } }

            impl<T: ?Sized> AmbiguousIfImpl<()> for Check<T> { }
            impl<T: ?Sized $(+ $t)*> AmbiguousIfImpl<u8> for Check<T> { }

            <Check::<$x> as AmbiguousIfImpl<_>>::some_item()
        };
    };
}

#[cfg(fusion)]
macro_rules! fuse_op {
    (
        $(<$($ty:ident: $trait:ident),* $(,)?> $name:ident( $($arg:ident: $arg_t:ty),* $(,)? ));* $(;)?
    ) => {
        #[allow(unused_imports)]
        use crate::{IourOpCode, PollOpCode, OpEntry, OpType, sys::prelude::*};

        ::paste::paste! {
            $(
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
                                        ::std::hint::unreachable_unchecked()
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
                                        ::std::hint::unreachable_unchecked()
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
                        use [< $name Inner >]::*;
                        match self.inner {
                            Uninit(..) => {
                                self.inner.poll();
                                self.into_inner()
                            },
                            Poll(op) => op.into_inner(),
                            IoUring(op) => op.into_inner(),
                        }
                    }
                }

                impl<$($ty: $trait),*> $name <$($ty),*> {
                    #[doc = concat!("Create a new `", stringify!($name), "`.")]
                    pub fn new($($arg: $arg_t),*) -> Self {
                        Self { inner: [< $name Inner >]::Uninit($($arg),*) }
                    }
                }

                unsafe impl<$($ty: $trait),*> PollOpCode for $name<$($ty),*> {
                    type Control = <poll::$name<$($ty),*> as PollOpCode>::Control;

                    unsafe fn init(&mut self, ctrl: &mut <Self as PollOpCode>::Control) {
                        unsafe { PollOpCode::init(self.inner.poll(), ctrl) }
                    }

                    fn pre_submit(&mut self, control: &mut <Self as PollOpCode>::Control) -> std::io::Result<crate::Decision> {
                        self.inner.poll().pre_submit(control)
                    }

                    fn op_type(&mut self, control: &mut <Self as PollOpCode>::Control) -> Option<OpType> {
                        self.inner.poll().op_type(control)
                    }

                    fn operate(
                        &mut self, control: &mut <Self as PollOpCode>::Control,
                    ) -> std::task::Poll<std::io::Result<usize>> {
                        self.inner.poll().operate(control)
                    }
                }

                unsafe impl<$($ty: $trait),*> IourOpCode for $name<$($ty),*> {
                    type Control = <iour::$name<$($ty),*> as IourOpCode>::Control;

                    unsafe fn init(&mut self, ctrl: &mut <Self as IourOpCode>::Control) {
                        unsafe { self.inner.iour().init(ctrl) }
                    }

                    fn create_entry(&mut self, control: &mut <Self as IourOpCode>::Control) -> OpEntry {
                        self.inner.iour().create_entry(control)
                    }

                    fn create_entry_fallback(&mut self, control: &mut <Self as IourOpCode>::Control) -> OpEntry {
                        self.inner.iour().create_entry_fallback(control)
                    }

                    fn call_blocking(&mut self, control: &mut <Self as IourOpCode>::Control) -> std::io::Result<usize> {
                        self.inner.iour().call_blocking(control)
                    }

                    unsafe fn set_result(&mut self, control: &mut <Self as IourOpCode>::Control, result: &std::io::Result<usize>, extra: &crate::Extra) {
                        unsafe { self.inner.iour().set_result(control, result, extra) }
                    }

                    unsafe fn push_multishot(&mut self, control: &mut <Self as IourOpCode>::Control, result: std::io::Result<usize>, extra: crate::Extra) {
                        unsafe { self.inner.iour().push_multishot(control, result, extra) }
                    }

                    fn pop_multishot(&mut self, control: &mut <Self as IourOpCode>::Control) -> Option<BufResult<usize, crate::Extra>> {
                        self.inner.iour().pop_multishot(control)
                    }
                }
            )*
        }
    };
}

#[cfg(fusion)]
pub(crate) use fuse_op;
