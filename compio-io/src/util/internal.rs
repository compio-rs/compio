use std::mem::MaybeUninit;

use compio_buf::{IoBufMut, SetLenExt};

#[inline]
pub(crate) fn slice_to_uninit(src: &[u8], dst: &mut [MaybeUninit<u8>]) -> usize {
    let len = src.len().min(dst.len());
    dst[..len].write_copy_of_slice(&src[..len]);
    len
}

/// Copy the contents of a slice into a buffer implementing [`IoBufMut`].
#[inline]
pub(crate) fn slice_to_buf<B: IoBufMut + ?Sized>(src: &[u8], buf: &mut B) -> usize {
    let len = slice_to_uninit(src, buf.as_uninit());
    unsafe { buf.advance_to(len) };

    len
}

pub(crate) const DEFAULT_BUF_SIZE: usize = 8 * 1024;
pub(crate) const MISSING_BUF: &str = "The buffer was submitted for io and never returned";
