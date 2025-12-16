#![allow(dead_code)]

use std::pin::Pin;

use compio_buf::{IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};

#[cfg(unix)]
mod sys {
    use std::mem::MaybeUninit;

    pub use libc::iovec as Inner;

    pub fn new(ptr: *mut MaybeUninit<u8>, len: usize) -> Inner {
        Inner {
            iov_base: ptr as *mut libc::c_void,
            iov_len: len,
        }
    }
}

#[cfg(windows)]
mod sys {
    use std::mem::MaybeUninit;

    pub use windows_sys::Win32::Networking::WinSock::WSABUF as Inner;

    pub fn new(ptr: *mut MaybeUninit<u8>, len: usize) -> Inner {
        Inner {
            len: len as u32,
            buf: ptr as _,
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod sys {
    pub type Inner = ();

    pub fn new(_: *mut MaybeUninit<u8>, _: usize) -> Inner {
        unreachable!("SysSlice will not be used on platforms other than unix and windows")
    }
}

/// A ptr + length combination to interact with system API.
///
/// Like [`IoSlice`] and [`IoSliceMut`] in `std`, `SysSlice` guarantees the ABI
/// compatibility on unix and windows, but without the lifetime, makes it easier
/// to use with compio driver.
///
/// To construct a `SysSlice`, use the extension traits. It can only be
/// constructed either from:
/// - a null ptr + 0 length with [`SysSlice::null()`]
/// - a pinned `IoBuf` (for initialized memory)
/// - a pinned `IoBufMut` (for uninitialized memory)
///
/// [`IoSlice`]: std::io::IoSlice
/// [`IoSliceMut`]: std::io::IoSliceMut
#[repr(transparent)]
pub(crate) struct SysSlice(sys::Inner);

impl SysSlice {
    pub fn null() -> Self {
        Self(sys::new(std::ptr::null_mut(), 0))
    }

    fn from_slice(slice: &[u8]) -> Self {
        Self(sys::new(
            slice.as_ptr() as *mut std::mem::MaybeUninit<u8>,
            slice.len(),
        ))
    }

    fn from_uninit(value: &mut [std::mem::MaybeUninit<u8>]) -> Self {
        Self(sys::new(value.as_mut_ptr(), value.len()))
    }

    pub fn into_inner(self) -> sys::Inner {
        self.0
    }

    pub fn ptr(&self) -> *mut std::mem::MaybeUninit<u8> {
        #[cfg(unix)]
        {
            self.0.iov_base as *mut std::mem::MaybeUninit<u8>
        }
        #[cfg(windows)]
        {
            self.0.buf as *mut std::mem::MaybeUninit<u8>
        }
    }

    pub fn len(&self) -> usize {
        #[cfg(unix)]
        {
            self.0.iov_len
        }
        #[cfg(windows)]
        {
            self.0.len as usize
        }
    }
}

pub(crate) trait IoBufExt: IoBuf {
    /// Convert a pinned [`IoBuf`] into a [`SysSlice`].
    ///
    /// This will only include initialized memory.
    fn sys_slice(self: Pin<&Self>) -> SysSlice {
        SysSlice::from_slice(self.as_slice())
    }
}

impl<T: IoBuf + ?Sized> IoBufExt for T {}

pub(crate) trait IoBufMutExt: IoBufMut {
    /// Convert a pinned [`IoBufMut`] into a [`SysSlice`].
    ///
    /// This will include uninitialized memory.
    fn sys_slice_mut(self: Pin<&mut Self>) -> SysSlice {
        // SAFETY:
        // - we're not moving the buffer, and
        // - creating a `SysSlice` is like calling `as_ptr`, as long as we're not
        //   dereferencing it, it's safe. It's up to the consumer of `SysSlice` to
        //   ensure the pointer is used safely.
        let slice = unsafe { self.get_unchecked_mut() };
        SysSlice::from_uninit(slice.as_uninit())
    }
}

impl<T: IoBufMut + ?Sized> IoBufMutExt for T {}

pub(crate) trait IoVectoredBufExt: IoVectoredBuf {
    /// Convert a pinned [`IoVectoredBuf`] into a vector of [`SysSlice`]s.
    fn sys_slices(self: Pin<&Self>) -> Vec<SysSlice> {
        self.iter_slice().map(SysSlice::from_slice).collect()
    }
}

impl<T: IoVectoredBuf + ?Sized> IoVectoredBufExt for T {}

pub(crate) trait IoVectoredBufMutExt: IoVectoredBufMut {
    /// Convert a pinned [`IoVectoredBufMut`] into a vector of [`SysSlice`]s.
    fn sys_slices_mut(self: Pin<&mut Self>) -> Vec<SysSlice> {
        // SAFETY: Similar to `IoBufMutExt::sys_slice`
        let this = unsafe { self.get_unchecked_mut() };
        this.iter_uninit_slice()
            .map(SysSlice::from_uninit)
            .collect()
    }
}

impl<T: IoVectoredBufMut + ?Sized> IoVectoredBufMutExt for T {}
