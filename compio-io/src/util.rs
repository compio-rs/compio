use std::mem::MaybeUninit;

use compio_buf::IoBufMut;

#[inline]
fn copy(src: &[u8], dst: &mut [MaybeUninit<u8>]) -> usize {
    let len = src.len().min(dst.len());
    unsafe {
        std::ptr::copy_nonoverlapping(src.as_ptr() as _, dst.as_mut_ptr(), len);
    }
    len
}

/// Copy the contents of a slice into a buffer implementing [`IoBufMut`].
#[inline]
pub(crate) fn slice_to_buf(src: &[u8], buf: &mut impl IoBufMut) -> usize {
    let len = copy(src, buf.as_uninit_slice());
    unsafe { buf.set_buf_init(len) };

    len
}
