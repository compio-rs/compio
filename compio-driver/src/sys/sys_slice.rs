#![allow(dead_code)]

use compio_buf::{IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        use std::mem::MaybeUninit;
        use libc::iovec as Inner;

        fn new(ptr: *mut MaybeUninit<u8>, len: usize) -> Inner {
            Inner {
                iov_base: ptr as *mut libc::c_void,
                iov_len: len,
            }
        }
    } else if #[cfg(windows)] {
        use std::mem::MaybeUninit;
        use windows_sys::Win32::Networking::WinSock::WSABUF as Inner;

        fn new(ptr: *mut MaybeUninit<u8>, len: usize) -> Inner {
            Inner {
                len: len.try_into().unwrap_or(u32::MAX),
                buf: ptr as _,
            }
        }
    } else {
        type Inner = ();

        fn new(_: *mut MaybeUninit<u8>, _: usize) -> Inner {
            unreachable!("SysSlice will not be used on platforms other than unix and windows")
        }
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
/// - a `IoBuf` (for initialized memory)
/// - a `IoBufMut` (for uninitialized memory)
///
/// [`IoSlice`]: std::io::IoSlice
/// [`IoSliceMut`]: std::io::IoSliceMut
#[repr(transparent)]
pub(crate) struct SysSlice(Inner);

impl Default for SysSlice {
    fn default() -> Self {
        Self::null()
    }
}

impl SysSlice {
    pub fn null() -> Self {
        Self(new(std::ptr::null_mut(), 0))
    }

    fn from_slice(slice: &[u8]) -> Self {
        Self(new(
            slice.as_ptr() as *mut std::mem::MaybeUninit<u8>,
            slice.len(),
        ))
    }

    fn from_uninit(value: &mut [std::mem::MaybeUninit<u8>]) -> Self {
        Self(new(value.as_mut_ptr(), value.len()))
    }

    pub fn into_inner(self) -> Inner {
        self.0
    }

    pub fn ptr(&self) -> *mut std::mem::MaybeUninit<u8> {
        #[cfg(unix)]
        return self.0.iov_base as *mut std::mem::MaybeUninit<u8>;
        #[cfg(windows)]
        return self.0.buf as *mut std::mem::MaybeUninit<u8>;
    }

    pub fn len(&self) -> usize {
        #[cfg(unix)]
        return self.0.iov_len;
        #[cfg(windows)]
        return self.0.len as usize;
    }
}

pub(crate) trait IoBufExt: IoBuf {
    /// Convert a pinned [`IoBuf`] into a [`SysSlice`].
    ///
    /// This will only include initialized memory.
    fn sys_slice(&self) -> SysSlice {
        SysSlice::from_slice(self.as_init())
    }
}

impl<T: IoBuf + ?Sized> IoBufExt for T {}

pub(crate) trait IoBufMutExt: IoBufMut {
    /// Convert a pinned [`IoBufMut`] into a [`SysSlice`].
    ///
    /// This will include uninitialized memory.
    fn sys_slice_mut(&mut self) -> SysSlice {
        SysSlice::from_uninit(self.as_uninit())
    }
}

impl<T: IoBufMut + ?Sized> IoBufMutExt for T {}

pub(crate) trait IoVectoredBufExt: IoVectoredBuf {
    /// Convert a pinned [`IoVectoredBuf`] into a vector of [`SysSlice`]s.
    fn sys_slices(&self) -> Vec<SysSlice> {
        self.iter_slice().map(SysSlice::from_slice).collect()
    }
}

impl<T: IoVectoredBuf + ?Sized> IoVectoredBufExt for T {}

pub(crate) trait IoVectoredBufMutExt: IoVectoredBufMut {
    /// Convert a pinned [`IoVectoredBufMut`] into a vector of [`SysSlice`]s.
    fn sys_slices_mut(&mut self) -> Vec<SysSlice> {
        self.iter_uninit_slice()
            .map(SysSlice::from_uninit)
            .collect()
    }
}

impl<T: IoVectoredBufMut + ?Sized> IoVectoredBufMutExt for T {}
