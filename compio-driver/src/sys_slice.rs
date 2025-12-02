use compio_buf::{IoBuffer, IoBufferMut, IoVectoredBuf, IoVectoredBufMut};

#[cfg(unix)]
mod sys {
    use std::mem::MaybeUninit;

    #[repr(transparent)]
    pub struct Inner(libc::iovec);

    impl Inner {
        pub fn new(ptr: *mut MaybeUninit<u8>, len: usize) -> Self {
            Self(libc::iovec {
                iov_base: ptr as *mut libc::c_void,
                iov_len: len,
            })
        }
    }
}

#[cfg(windows)]
mod sys {
    use std::mem::MaybeUninit;

    use windows_sys::Win32::Networking::WinSock::WSABUF;

    #[repr(transparent)]
    pub struct Inner(WSABUF);

    impl Inner {
        pub fn new(ptr: *mut MaybeUninit<u8>, len: usize) -> Self {
            Self(WSABUF {
                len: len as u32,
                buf: ptr as _,
            })
        }
    }
}

#[cfg(not(any(unix, windows)))]
compile_error!("`SysSlice` is only available on unix and windows");

/// An unsafe, `'static`, initialized, and immutable slice of bytes to interact
/// with system API.
///
/// Like [`IoSlice`] in `std`, `SysSlice` guarantees the ABI compatibility
/// on unix and windows, but without the lifetime, makes it easier to use with
/// compio driver at the cost of unsafe to construct. `SysSlice` should only be
/// used with compio driver.
///
/// [`IoSlice`]: std::io::IoSlice
#[repr(transparent)]
pub struct SysSlice(sys::Inner);

impl From<IoBuffer> for SysSlice {
    fn from(value: IoBuffer) -> Self {
        let (ptr, len) = value.into_raw_parts();
        Self(sys::Inner::new(ptr as _, len))
    }
}

/// An unsafe, `'static`, maybe uninitialized, and mutable slice of bytes to
/// interact with system API.
///
/// Like [`IoSliceMut`] in `std`, [`SysSliceMut`] guarantees the ABI
/// compatibility on unix and windows, but without the lifetime and accepts
/// [`MaybeUninit<u8>`], makes it easier to use with compio driver at the cost
/// of unsafe to construct. [`SysSliceMut`] should only be used with compio
/// driver.
///
/// [`IoSliceMut`]: std::io::IoSliceMut
#[repr(transparent)]
pub struct SysSliceMut(sys::Inner);

impl From<IoBufferMut> for SysSliceMut {
    fn from(value: IoBufferMut) -> Self {
        let (ptr, len) = value.into_raw_parts();
        Self(sys::Inner::new(ptr, len))
    }
}

pub trait IoVectoredBufExt: IoVectoredBuf {
    /// Convert the vectored buffer into a vector of [`SysSlice`]s.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the underlying buffers are valid for the
    /// lifetime of the returned [`SysSlice`]s, and that they are not used for
    /// mutating while the [`SysSlice`]s are in use.
    unsafe fn sys_slices(&self) -> Vec<SysSlice> {
        unsafe { self.iter_buffer() }.map(SysSlice::from).collect()
    }
}

impl<T: IoVectoredBuf + ?Sized> IoVectoredBufExt for T {}

pub trait IoVectoredBufMutExt: IoVectoredBufMut {
    /// Convert the vectored buffer into a vector of [`SysSliceMut`]s.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the underlying buffers are valid for the
    /// lifetime of the returned [`SysSliceMut`]s, and that they are not used
    /// anywhere else while the [`SysSliceMut`]s are in use.
    unsafe fn sys_slices_mut(&mut self) -> Vec<SysSliceMut> {
        unsafe { self.iter_buffer_mut() }
            .map(SysSliceMut::from)
            .collect()
    }
}

impl<T: IoVectoredBufMut + ?Sized> IoVectoredBufMutExt for T {}
