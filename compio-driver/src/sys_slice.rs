use compio_buf::{IoVectoredBuf, IoVectoredBufMut};

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

/// An unsafely `'static`, maybe-uninitialized slice of bytes to interact
/// with system API.
///
/// Like [`IoSlice`] and [`IoSliceMut`] in `std`, `SysSlice` guarantees the ABI
/// compatibility on unix and windows, but without the lifetime, makes it easier
/// to use with compio driver at the cost of unsafe to construct.
///
/// [`IoSlice`]: std::io::IoSlice
/// [`IoSliceMut`]: std::io::IoSliceMut
#[repr(transparent)]
pub(crate) struct SysSlice(sys::Inner);

#[allow(dead_code)]
impl SysSlice {
    pub fn into_inner(self) -> sys::Inner {
        self.0
    }
}

impl From<&[u8]> for SysSlice {
    fn from(value: &[u8]) -> Self {
        Self(sys::new(
            value.as_ptr() as *mut std::mem::MaybeUninit<u8>,
            value.len(),
        ))
    }
}

impl From<&mut [std::mem::MaybeUninit<u8>]> for SysSlice {
    fn from(value: &mut [std::mem::MaybeUninit<u8>]) -> Self {
        Self(sys::new(value.as_mut_ptr(), value.len()))
    }
}

impl From<&mut [u8]> for SysSlice {
    fn from(value: &mut [u8]) -> Self {
        Self(sys::new(
            value.as_mut_ptr() as *mut std::mem::MaybeUninit<u8>,
            value.len(),
        ))
    }
}

pub(crate) trait IoVectoredBufExt: IoVectoredBuf {
    /// Convert the vectored buffer into a vector of [`SysSlice`]s.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the underlying buffers are valid for the
    /// lifetime of the returned [`SysSlice`]s, and that they are not used for
    /// mutating while the [`SysSlice`]s are in use.
    unsafe fn sys_slices(&self) -> Vec<SysSlice> {
        self.iter_slice().map(SysSlice::from).collect()
    }
}

impl<T: IoVectoredBuf + ?Sized> IoVectoredBufExt for T {}

pub(crate) trait IoVectoredBufMutExt: IoVectoredBufMut {
    /// Convert the vectored buffer into a vector of [`SysSlice`]s.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the underlying buffers are valid for the
    /// lifetime of the returned [`SysSlice`]s, and that they are not used
    /// anywhere else while the [`SysSlice`]s are in use.
    unsafe fn sys_slices_mut(&mut self) -> Vec<SysSlice> {
        self.iter_uninit_slice().map(SysSlice::from).collect()
    }
}

impl<T: IoVectoredBufMut + ?Sized> IoVectoredBufMutExt for T {}
